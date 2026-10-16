//! Where the browser is sent back to after signing in to a server: a port
//! on this computer that takes one redirect (RFC 8252, a loopback redirect
//! for a native app; the MCP authorization specification allows a
//! `localhost` redirect).

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

/// The most of a request read: a redirect is one short line and a few headers.
const MOST_BYTES: usize = 16 * 1024;

pub struct Loopback {
    listener: TcpListener,
    port: u16,
}

/// What the browser came back with.
pub struct Back {
    pub code: String,
    pub state: String,
    /// The authorization server that answered, where it says (RFC 9207).
    pub issuer: Option<String>,
}

impl Loopback {
    pub async fn open() -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| format!("no port could be opened for the browser to come back to: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();

        Ok(Self { listener, port })
    }

    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}/callback", self.port)
    }

    /// Wait for the browser to come back. Requests for anything else, a
    /// browser's guess at an icon say, are answered and waited past.
    pub async fn wait(self, wait: Duration) -> Result<Back, String> {
        let take = async {
            loop {
                let Ok((mut stream, _)) = self.listener.accept().await else {
                    continue;
                };
                let Some(target) = request_target(&mut stream).await else {
                    continue;
                };
                let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
                    respond(&mut stream, "400 Bad Request", "That was not a sign-in.").await;
                    continue;
                };
                if url.path() != "/callback" {
                    respond(&mut stream, "404 Not Found", "Nothing here.").await;
                    continue;
                }
                let get = |name: &str| {
                    url.query_pairs()
                        .find(|(k, _)| k == name)
                        .map(|(_, v)| v.into_owned())
                };
                if let Some(error) = get("error") {
                    respond(
                        &mut stream,
                        "200 OK",
                        "Signing in did not work. You can close this tab and go back to Brolga.",
                    )
                    .await;
                    let said = get("error_description").unwrap_or_default();
                    return Err(format!("the server said {error}: {said}"));
                }
                match (get("code"), get("state")) {
                    (Some(code), Some(state)) => {
                        respond(
                            &mut stream,
                            "200 OK",
                            "Signed in. You can close this tab and go back to Brolga.",
                        )
                        .await;
                        return Ok(Back {
                            code,
                            state,
                            issuer: get("iss"),
                        });
                    }
                    _ => respond(&mut stream, "400 Bad Request", "That was not a sign-in.").await,
                }
            }
        };

        tokio::time::timeout(wait, take)
            .await
            .map_err(|_| "the browser did not come back in time".to_owned())?
    }
}

/// The path and query of one HTTP request.
async fn request_target(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buffer.windows(4).any(|w| w == b"\r\n\r\n") && buffer.len() < MOST_BYTES {
        let read = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut chunk))
            .await
            .ok()?
            .ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let text = String::from_utf8_lossy(&buffer);
    let mut first = text.lines().next()?.split(' ');
    match (first.next(), first.next()) {
        (Some("GET"), Some(target)) => Some(target.to_owned()),
        _ => None,
    }
}

async fn respond(stream: &mut TcpStream, status: &str, message: &str) {
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Brolga</title><p style=\"font-family:sans-serif\">{message}</p>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}
