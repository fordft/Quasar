//! Loopback HTTP fixtures compile only in unit tests. No real OpenAI account.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub(crate) struct Server {
    pub base: String,
    pub requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    pub async fn start(responses: Vec<(u16, serde_json::Value)>) -> Self {
        Self::delayed(responses, std::time::Duration::ZERO).await
    }
    pub async fn delayed(
        responses: Vec<(u16, serde_json::Value)>,
        delay: std::time::Duration,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fixture");
        let base = format!("http://{}", listener.local_addr().expect("fixture port"));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let task = tokio::spawn(async move {
            let mut responses: VecDeque<_> = responses.into();
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut bytes = Vec::new();
                let mut chunk = [0; 2048];
                loop {
                    let read = stream.read(&mut chunk).await.expect("fixture read");
                    if read == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..read]);
                    assert!(bytes.len() < 1024 * 1024, "fixture request bounded");
                    if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                        let headers = std::str::from_utf8(&bytes[..end]).expect("headers");
                        let length: usize = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|s| s.trim().parse().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                captured
                    .lock()
                    .expect("requests")
                    .push(String::from_utf8(bytes).expect("request"));
                tokio::time::sleep(delay).await;
                let (status, body) = responses
                    .pop_front()
                    .unwrap_or((500, serde_json::json!({})));
                let body = serde_json::to_string(&body).expect("fixture json");
                let reply = format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        Self {
            base,
            requests,
            task,
        }
    }
}
