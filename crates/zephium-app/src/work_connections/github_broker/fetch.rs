//! Fixed-origin public GETs. The checked DNS addresses reach the connector;
//! ambient proxies, redirects, cookies and provider credentials are not admitted.
use super::{Failure, Request};
use crate::work_commands::runner::worker_slots::{self, Slots};
use reqwest::{redirect::Policy, Client};
use std::{
    net::ToSocketAddrs,
    sync::{Arc, OnceLock},
    time::Duration,
};
const HOST: &str = "api.github.com";
const LIMIT: usize = 60 * 1024;
struct Resolver;
impl reqwest::dns::Resolve for Resolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            if host != HOST {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "unexpected broker host",
                )
                .into());
            }
            resolve_with(resolver_slots(), move || {
                checked_addresses((host.as_str(), 0).to_socket_addrs()?)
            })
            .await
            .map_err(Into::into)
        })
    }
}
fn resolver_slots() -> &'static Arc<Slots> {
    static SLOTS: OnceLock<Arc<Slots>> = OnceLock::new();
    SLOTS.get_or_init(|| Arc::new(Slots::new(4)))
}
async fn resolve_with(
    slots: &Arc<Slots>,
    resolve: impl FnOnce() -> Result<reqwest::dns::Addrs, std::io::Error> + Send + 'static,
) -> Result<reqwest::dns::Addrs, std::io::Error> {
    // Dropping a DNS future cannot cancel getaddrinfo. Its physical worker owns
    // capacity until return, including after the request or Work is cancelled.
    let permit = slots.reserve().map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "broker resolution capacity occupied",
        )
    })?;
    worker_slots::launch(permit, resolve, |_| true)
        .await
        .map_err(|_| std::io::Error::other("broker resolution did not settle"))?
}
fn checked_addresses(
    addresses: impl Iterator<Item = std::net::SocketAddr>,
) -> Result<reqwest::dns::Addrs, std::io::Error> {
    let mut kept = Vec::new();
    for address in addresses.take(33) {
        if kept.len() == 32 || !zephium_agentic::public_asset::public_address(address.ip()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "non-public broker address or resolution limit",
            ));
        }
        kept.push(address);
    }
    if kept.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "empty broker resolution",
        ));
    }
    Ok(Box::new(kept.into_iter()))
}
fn client(resolver: Arc<dyn reqwest::dns::Resolve>) -> Result<Client, Failure> {
    Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .dns_resolver(resolver)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .user_agent("Quasar-Work-GitHub-CLI")
        .build()
        .map_err(|_| Failure::Failed)
}
pub(super) async fn get(request: &Request) -> Result<Vec<u8>, Failure> {
    let client = client(Arc::new(Resolver))?;
    receive(
        client
            .get(format!("https://{HOST}{}", request.endpoint()))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|_| Failure::Failed)?,
    )
    .await
}
async fn receive(mut response: reqwest::Response) -> Result<Vec<u8>, Failure> {
    if response.status() != reqwest::StatusCode::OK
        || response
            .content_length()
            .is_some_and(|length| length > LIMIT as u64)
        || response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_none_or(|value| value.split(';').next() != Some("application/json"))
    {
        return Err(Failure::Failed);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Failure::Failed)? {
        if bytes.len() + chunk.len() > LIMIT {
            return Err(Failure::Limit);
        }
        bytes.extend_from_slice(&chunk);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| Failure::Failed)?;
    if zephium_agentic::contains_recognized_secret(text) {
        return Err(Failure::Sensitive);
    }
    // Scan decoded JSON too, before clipping or discarding fields. Escaped
    // recognizable token/key shapes must not reach either child or model.
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| Failure::Failed)?;
    let normalized = serde_json::to_vec(&value).map_err(|_| Failure::Failed)?;
    if zephium_agentic::contains_recognized_secret(
        std::str::from_utf8(&normalized).map_err(|_| Failure::Failed)?,
    ) {
        return Err(Failure::Sensitive);
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancelled_resolution_holds_capacity_until_physical_return() {
        let slots = Arc::new(Slots::new(1));
        let owned = slots.clone();
        let (started, observed) = tokio::sync::oneshot::channel();
        let (release, held) = std::sync::mpsc::channel();
        let caller = tokio::spawn(async move {
            resolve_with(&owned, move || {
                let _ = started.send(());
                let _ = held.recv();
                Err(std::io::Error::other("inert resolved failure"))
            })
            .await
        });
        observed.await.unwrap();
        caller.abort();
        assert!(matches!(caller.await, Err(error) if error.is_cancelled()));
        assert_eq!(slots.outstanding(), 1);
        let refused = resolve_with(&slots, || {
            panic!("capacity refusal must not start another resolver")
        })
        .await;
        assert!(matches!(refused,Err(ref error) if error.kind()==std::io::ErrorKind::WouldBlock));
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while slots.outstanding() != 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            slots.reserve().is_ok(),
            "capacity returns only after resolver acknowledgment"
        );
    }

    use std::net::{IpAddr, SocketAddr};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    #[test]
    fn actual_connector_addresses_reject_private_mixed_empty_and_unbounded_dns() {
        let addr = |text: &str| SocketAddr::new(text.parse::<IpAddr>().unwrap(), 0);
        for addresses in [
            vec![],
            vec![addr("127.0.0.1")],
            vec![addr("1.1.1.1"), addr("10.0.0.1")],
            vec![addr("1.1.1.1"); 33],
        ] {
            assert!(checked_addresses(addresses.into_iter()).is_err());
        }
        assert!(checked_addresses(vec![addr("1.1.1.1")].into_iter()).is_ok());
    }
    async fn response(status: &str, headers: &str, body: &[u8]) -> reqwest::Response {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let reply=format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n",body.len()).into_bytes().into_iter().chain(body.iter().copied()).collect::<Vec<_>>();
        let server = async {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).await.unwrap();
            stream.write_all(&reply).await.unwrap();
        };
        let client = client(Arc::new(Resolver)).unwrap();
        let read = client.get(format!("http://127.0.0.1:{port}/inert")).send();
        let (_, response) = tokio::join!(server, read);
        response.unwrap()
    }
    #[tokio::test]
    async fn public_fetch_refuses_redirects_oversized_and_escaped_credentials() {
        let other = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let header = format!(
            "Location: http://127.0.0.1:{}/private\r\n",
            other.local_addr().unwrap().port()
        );
        assert!(receive(response("302 Found", &header, b"{}").await)
            .await
            .is_err());
        assert!(
            tokio::time::timeout(Duration::from_millis(30), other.accept())
                .await
                .is_err()
        );
        assert!(
            receive(response("200 OK", "", &vec![b'x'; LIMIT + 1]).await)
                .await
                .is_err()
        );
        assert!(matches!(
            receive(response("200 OK", "", br#"{"body":"\u0067hp_inertFixtureToken"}"#).await)
                .await,
            Err(Failure::Sensitive)
        ));
        assert_eq!(
            receive(response("200 OK", "", br#"{"body":"Public fixture"}"#).await)
                .await
                .unwrap(),
            br#"{"body":"Public fixture"}"#
        );
    }
    struct InertResolver(SocketAddr);
    impl reqwest::dns::Resolve for InertResolver {
        fn resolve(&self, _: reqwest::dns::Name) -> reqwest::dns::Resolving {
            let address = self.0;
            Box::pin(async move { Ok(Box::new(std::iter::once(address)) as reqwest::dns::Addrs) })
        }
    }
    #[test]
    fn inherited_proxy_cannot_redirect_the_broker_connection() {
        let _serial = crate::WORK_RUNTIME_TEST_SERIAL.lock().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let proxy=TcpListener::bind("127.0.0.1:0").await.unwrap();
            let target=TcpListener::bind("127.0.0.1:0").await.unwrap();
            struct Restore(Vec<(&'static str,Option<std::ffi::OsString>)>);
            impl Drop for Restore {fn drop(&mut self){for (name,value) in self.0.drain(..){if let Some(value)=value{std::env::set_var(name,value)}else{std::env::remove_var(name)}}}}
            let _restore=Restore(["HTTP_PROXY","HTTPS_PROXY","ALL_PROXY","http_proxy","https_proxy","all_proxy","NO_PROXY","no_proxy"].into_iter().map(|name|(name,std::env::var_os(name))).collect());
            let proxy_url=format!("http://127.0.0.1:{}",proxy.local_addr().unwrap().port());
            for name in ["HTTP_PROXY","HTTPS_PROXY","ALL_PROXY","http_proxy","https_proxy","all_proxy"]{std::env::set_var(name,&proxy_url);}
            for name in ["NO_PROXY","no_proxy"]{std::env::set_var(name,"");}
            let client=client(Arc::new(InertResolver(target.local_addr().unwrap()))).unwrap();
            let read=client.get(format!("http://api.github.com:{}/inert",target.local_addr().unwrap().port())).send();
            let server=async {let (mut stream,_)=target.accept().await.unwrap();let mut bytes=[0;4096];let _=stream.read(&mut bytes).await.unwrap();stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").await.unwrap();};
            let (_,response)=tokio::time::timeout(Duration::from_secs(2),async{tokio::join!(server,read)}).await.unwrap();
            assert!(response.unwrap().status().is_success());
            assert!(tokio::time::timeout(Duration::from_millis(30),proxy.accept()).await.is_err());
        });
    }
}
