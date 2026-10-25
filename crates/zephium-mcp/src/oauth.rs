//! Streamable HTTP servers: open, with a bearer token, or with OAuth. Sign-in
//! is the standard authorization-code flow with PKCE: the person approves in
//! their browser and the redirect comes back to a one-shot listener on the
//! loopback address. Tokens live wherever the caller's store keeps them.
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::ClientConfig;
use rmcp::service::{ClientLifecycleMode, ClientServiceExt, RunningService};
use rmcp::transport::auth::{
    AuthClient, AuthError, AuthorizationManager, AuthorizationRequest, CredentialStore, OAuthState,
    StoredCredentials,
};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::RoleClient;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::McpError;

/// Largest server-sent event accepted from a server.
const MAX_EVENT_BYTES: usize = 8 * 1024 * 1024;
const SIGN_IN_PATIENCE: Duration = Duration::from_secs(300);

/// Where one server's OAuth credentials are kept, as opaque JSON.
pub trait TokenStore: Send + Sync {
    fn load(&self) -> Option<String>;
    fn save(&self, value: &str) -> bool;
    fn clear(&self);
}

/// Ephemeral tokens for draft validation and tests. Never touches the Keychain.
#[derive(Default)]
pub struct MemoryTokens(pub std::sync::Mutex<Option<String>>);
impl TokenStore for MemoryTokens {
    fn load(&self) -> Option<String> {
        self.0.lock().ok()?.clone()
    }
    fn save(&self, value: &str) -> bool {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(value.to_owned());
            true
        } else {
            false
        }
    }
    fn clear(&self) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = None;
        }
    }
}

pub enum HttpAuth {
    None,
    Bearer(String),
    OAuth(Arc<dyn TokenStore>),
}

pub struct HttpServer {
    /// An `https` URL, or `http` on the loopback address.
    pub url: String,
    pub auth: HttpAuth,
}

struct Store(Arc<dyn TokenStore>);

#[async_trait::async_trait]
impl CredentialStore for Store {
    async fn acquire_refresh_guard(
        &self,
    ) -> Result<Option<rmcp::transport::auth::CredentialRefreshGuard>, AuthError> {
        static REFRESH: std::sync::OnceLock<Arc<tokio::sync::Mutex<()>>> =
            std::sync::OnceLock::new();
        let lock = REFRESH.get_or_init(Default::default).clone();
        Ok(Some(rmcp::transport::auth::CredentialRefreshGuard::new(
            lock.lock_owned().await,
        )))
    }
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        let store = self.0.clone();
        let value = tokio::task::spawn_blocking(move || store.load())
            .await
            .map_err(|_| AuthError::InternalError("store".into()))?;
        Ok(value.and_then(|v| serde_json::from_str(&v).ok()))
    }
    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let value = serde_json::to_string(&credentials)
            .map_err(|_| AuthError::InternalError("store".into()))?;
        let store = self.0.clone();
        let saved = tokio::task::spawn_blocking(move || store.save(&value))
            .await
            .unwrap_or(false);
        if saved {
            Ok(())
        } else {
            Err(AuthError::InternalError("store".into()))
        }
    }
    async fn clear(&self) -> Result<(), AuthError> {
        let store = self.0.clone();
        let _ = tokio::task::spawn_blocking(move || store.clear()).await;
        Ok(())
    }
}

/// Only `https`, or plain `http` to this Mac.
pub fn valid_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let loopback = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    parsed.username().is_empty()
        && parsed.password().is_none()
        && url.len() <= 2048
        && (parsed.scheme() == "https" || (parsed.scheme() == "http" && loopback))
}

fn config(url: &str) -> StreamableHttpClientTransportConfig {
    let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_owned());
    config.max_sse_event_size = MAX_EVENT_BYTES;
    config
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .user_agent(concat!("Quasar/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

/// Whether the server turns away a request without credentials.
async fn asks_for_sign_in(url: &str, bearer: Option<&str>) -> bool {
    let mut request = http_client()
        .post(url)
        .header("accept", "application/json, text/event-stream")
        .header("content-type", "application/json")
        .body(r#"{"jsonrpc":"2.0","id":0,"method":"ping"}"#);
    if let Some(token) = bearer {
        request = request.bearer_auth(token);
    }
    matches!(
        tokio::time::timeout(Duration::from_secs(10), request.send()).await,
        Ok(Ok(response)) if response.status() == reqwest::StatusCode::UNAUTHORIZED
    )
}

impl HttpServer {
    pub(crate) async fn connect(
        &self,
        client: ClientConfig,
        lifecycle: ClientLifecycleMode,
        limit: Duration,
    ) -> Result<RunningService<RoleClient, ClientConfig>, McpError> {
        tokio::time::timeout(limit, self.connect_inner(client, lifecycle, limit))
            .await
            .map_err(|_| McpError::Timeout)?
    }

    async fn connect_inner(
        &self,
        client: ClientConfig,
        lifecycle: ClientLifecycleMode,
        limit: Duration,
    ) -> Result<RunningService<RoleClient, ClientConfig>, McpError> {
        if !valid_url(&self.url) {
            return Err(McpError::Protocol);
        }
        let served = match &self.auth {
            HttpAuth::None | HttpAuth::Bearer(_) => {
                let mut config = config(&self.url);
                if let HttpAuth::Bearer(token) = &self.auth {
                    config.auth_header = Some(token.clone());
                }
                let transport = StreamableHttpClientTransport::with_client(http_client(), config);
                tokio::time::timeout(limit, client.serve_with_lifecycle(transport, lifecycle)).await
            }
            HttpAuth::OAuth(store) => {
                let mut manager = AuthorizationManager::new(self.url.as_str())
                    .await
                    .map_err(|_| McpError::Closed)?;
                manager.set_credential_store(Store(store.clone()));
                if !manager
                    .initialize_from_store()
                    .await
                    .map_err(|_| McpError::Unauthorized)?
                {
                    return Err(McpError::Unauthorized);
                }
                // The SDK refreshes expiring tokens and persists rotations through Store.
                // A failed refresh becomes a recoverable sign-in state.
                manager
                    .get_access_token()
                    .await
                    .map_err(|_| McpError::Unauthorized)?;
                let transport = StreamableHttpClientTransport::with_client(
                    AuthClient::new(http_client(), manager),
                    config(&self.url),
                );
                tokio::time::timeout(limit, client.serve_with_lifecycle(transport, lifecycle)).await
            }
        };
        match served {
            Ok(Ok(service)) => Ok(service),
            Err(_) => Err(McpError::Timeout),
            Ok(Err(_)) => {
                let bearer = match &self.auth {
                    HttpAuth::Bearer(token) => Some(token.as_str()),
                    _ => None,
                };
                if matches!(self.auth, HttpAuth::OAuth(_))
                    || asks_for_sign_in(&self.url, bearer).await
                {
                    Err(McpError::Unauthorized)
                } else {
                    Err(McpError::Protocol)
                }
            }
        }
    }
}

/// Signs the person in to `url`. `open` shows the authorization page (a tab
/// in Zephium); the redirect is caught on the loopback address. The tokens
/// are saved through `store`.
pub async fn sign_in(
    url: &str,
    store: Arc<dyn TokenStore>,
    open: impl FnOnce(String) + Send,
) -> Result<(), McpError> {
    let (_cancel, receiver) = tokio::sync::watch::channel(false);
    sign_in_cancellable(url, store, open, receiver).await
}

/// Cancels the complete flow, including discovery, callback and token exchange.
/// Dropping the flow releases the loopback listener on every exit path.
pub async fn sign_in_cancellable(
    url: &str,
    store: Arc<dyn TokenStore>,
    open: impl FnOnce(String) + Send,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<(), McpError> {
    tokio::select! {
        biased;
        _ = async { if !*cancel.borrow() { let _ = cancel.changed().await; } } => Err(McpError::Cancelled),
        result = tokio::time::timeout(SIGN_IN_PATIENCE, sign_in_inner(url, store, open)) => result.map_err(|_| McpError::Timeout)?,
    }
}

async fn sign_in_inner(
    url: &str,
    store: Arc<dyn TokenStore>,
    open: impl FnOnce(String) + Send,
) -> Result<(), McpError> {
    if !valid_url(url) {
        return Err(McpError::Protocol);
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| McpError::Closed)?;
    let port = listener.local_addr().map_err(|_| McpError::Closed)?.port();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let mut manager = AuthorizationManager::new(url)
        .await
        .map_err(|_| McpError::Closed)?;
    manager.set_credential_store(Store(store));
    let mut state = OAuthState::Unauthorized(manager);
    state
        .start_authorization(AuthorizationRequest::new(redirect.clone()).with_client_name("Quasar"))
        .await
        .map_err(|_| McpError::Protocol)?;
    let page = state
        .get_authorization_url()
        .await
        .map_err(|_| McpError::Protocol)?;
    let expected_state = url::Url::parse(&page)
        .ok()
        .and_then(|url| {
            url.query_pairs()
                .find(|(key, _)| key == "state")
                .map(|(_, value)| value.into_owned())
        })
        .ok_or(McpError::Protocol)?;
    open(page);
    let callback = tokio::time::timeout(
        SIGN_IN_PATIENCE,
        catch_redirect(&listener, port, &expected_state),
    )
    .await
    .map_err(|_| McpError::Timeout)??;
    drop(listener);
    state
        .handle_callback_url(&callback)
        .await
        .map_err(|_| McpError::Unauthorized)
}

const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Returning to Quasar</title><style>body{font:15px -apple-system,system-ui,sans-serif;display:grid;place-items:center;height:90vh;margin:0;color:#1d1d1f;background:#fbfbfd}@media(prefers-color-scheme:dark){body{color:#f5f5f7;background:#161617}}p{margin:0}</style><p>Returning to Quasar…</p>";

/// Waits for the one redirect to `/callback` and answers it with a short page.
async fn catch_redirect(
    listener: &tokio::net::TcpListener,
    port: u16,
    expected_state: &str,
) -> Result<String, McpError> {
    loop {
        let (mut socket, _) = listener.accept().await.map_err(|_| McpError::Closed)?;
        let mut request = vec![0u8; 8192];
        let read =
            match tokio::time::timeout(Duration::from_secs(5), socket.read(&mut request)).await {
                Ok(Ok(read)) => read,
                _ => continue,
            };
        let head = String::from_utf8_lossy(&request[..read]);
        let Some(target) = head
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("GET "))
            .and_then(|rest| rest.split(' ').next())
        else {
            continue;
        };
        if !target.starts_with("/callback?") {
            let _ = socket
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                )
                .await;
            continue;
        }
        let callback = format!("http://127.0.0.1:{port}{target}");
        let Ok(parsed) = url::Url::parse(&callback) else {
            continue;
        };
        let states: Vec<_> = parsed
            .query_pairs()
            .filter(|(key, _)| key == "state")
            .collect();
        if states.len() != 1 || states[0].1 != expected_state {
            let _ = socket
                .write_all(
                    b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                )
                .await;
            continue;
        }
        let declined = parsed
            .query_pairs()
            .any(|(key, value)| key == "error" && value == "access_denied");
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n{DONE_PAGE}",
            DONE_PAGE.len()
        );
        let _ = socket.write_all(response.as_bytes()).await;
        return if declined {
            Err(McpError::Cancelled)
        } else {
            Ok(callback)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancellation_does_not_discover_or_open_a_page() {
        let (cancel, receiver) = tokio::sync::watch::channel(false);
        cancel.send(true).unwrap();
        assert_eq!(
            sign_in_cancellable(
                "https://example.invalid/mcp",
                Arc::new(MemoryTokens::default()),
                |_| panic!("must not open"),
                receiver
            )
            .await,
            Err(McpError::Cancelled)
        );
    }

    #[tokio::test]
    async fn dropping_callback_wait_releases_the_port() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let wait =
            tokio::spawn(async move { catch_redirect(&listener, address.port(), "xyz").await });
        wait.abort();
        let _ = wait.await;
        assert!(tokio::net::TcpListener::bind(address).await.is_ok());
    }

    #[tokio::test]
    async fn callback_timeout_releases_the_port() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(10), async move {
            catch_redirect(&listener, address.port(), "xyz").await
        })
        .await
        .is_err());
        assert!(tokio::net::TcpListener::bind(address).await.is_ok());
    }

    #[test]
    fn urls() {
        assert!(valid_url("https://mcp.linear.app/mcp"));
        assert!(valid_url("http://127.0.0.1:3000/mcp"));
        assert!(valid_url("http://localhost:8080/mcp"));
        for bad in [
            "http://example.com/mcp",
            "ftp://x",
            "https://user:pw@x.com",
            "not a url",
        ] {
            assert!(!valid_url(bad), "{bad}");
        }
    }

    #[tokio::test]
    async fn the_redirect_is_caught_once() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let caught = tokio::spawn(async move { catch_redirect(&listener, port, "xyz").await });
        let get = |path: &'static str| async move {
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            socket
                .write_all(format!("GET {path} HTTP/1.1\r\nhost: x\r\n\r\n").as_bytes())
                .await
                .unwrap();
            let mut answer = String::new();
            socket.read_to_string(&mut answer).await.unwrap();
            answer
        };
        assert!(get("/favicon.ico").await.starts_with("HTTP/1.1 404"));
        assert!(get("/callback?code=attacker&state=wrong")
            .await
            .starts_with("HTTP/1.1 400"));
        assert!(get("/callback?code=abc&state=xyz")
            .await
            .contains("Returning to Quasar"));
        assert_eq!(
            caught.await.unwrap().unwrap(),
            format!("http://127.0.0.1:{port}/callback?code=abc&state=xyz")
        );
    }
}
