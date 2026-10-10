//! Signing in through the person's own browser: the authorization code flow
//! with PKCE and a loopback redirect, as RFC 8252 describes for native apps.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

use crate::{AuthError, Registration, TokenClient, Tokens, pkce};

/// The most of a request the loopback listener will read. A redirect is one
/// short line and a few headers.
const MAX_REQUEST_BYTES: usize = 16 * 1024;

/// A sign-in that has been started and is waiting for the browser.
pub struct BrowserSignIn {
    /// The page to open in the system's default browser.
    pub url: String,
    registration: Registration,
    redirect_uri: String,
    state: String,
    verifier: String,
    listener: TcpListener,
    /// `localhost` may reach us over IPv6, so listen there too where we can.
    listener_v6: Option<TcpListener>,
}

/// What the browser was redirected back with.
enum Redirect {
    Code(String),
    Refused { error: String, description: String },
}

impl BrowserSignIn {
    /// Start a sign-in: open a loopback port and work out the page to send
    /// the browser to. `claims` carries a claims challenge from a 401, where
    /// one is being answered.
    pub async fn begin(
        client: &TokenClient,
        registration: &Registration,
        scopes: &[&str],
        login_hint: Option<&str>,
        claims: Option<&str>,
    ) -> Result<Self, AuthError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(AuthError::Loopback)?;
        let port = listener.local_addr().map_err(AuthError::Loopback)?.port();
        let listener_v6 = TcpListener::bind(("::1", port)).await.ok();

        // The registered redirect is `http://localhost`; the identity
        // platform ignores the port when matching a localhost redirect.
        let redirect_uri = format!("http://localhost:{port}");
        let state = pkce::random_token(16)?;
        let verifier = pkce::verifier()?;

        let mut url = Url::parse(&client.authority().authorize(&registration.tenant))
            .map_err(|e| AuthError::Protocol(format!("the sign-in address is not a URL: {e}")))?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("client_id", &registration.client_id)
                .append_pair("response_type", "code")
                .append_pair("redirect_uri", &redirect_uri)
                .append_pair("response_mode", "query")
                .append_pair("scope", &scopes.join(" "))
                .append_pair("state", &state)
                .append_pair("code_challenge", &pkce::challenge(&verifier))
                .append_pair("code_challenge_method", "S256")
                .append_pair("prompt", "select_account");
            if let Some(hint) = login_hint {
                query.append_pair("login_hint", hint);
            }
            if let Some(claims) = claims {
                query.append_pair("claims", claims);
            }
        }

        Ok(Self {
            url: url.into(),
            registration: registration.clone(),
            redirect_uri,
            state,
            verifier,
            listener,
            listener_v6,
        })
    }

    /// Wait for the browser to come back, then trade the code for tokens.
    pub async fn finish(self, client: &TokenClient, wait: Duration) -> Result<Tokens, AuthError> {
        let redirect = tokio::time::timeout(wait, self.redirect())
            .await
            .map_err(|_| AuthError::TimedOut)??;

        match redirect {
            Redirect::Code(code) => {
                client
                    .exchange_code(
                        &self.registration,
                        &code,
                        &self.redirect_uri,
                        &self.verifier,
                    )
                    .await
            }
            Redirect::Refused { error, description } => Err(AuthError::refused(
                &error,
                codes_in(&description),
                description,
            )),
        }
    }

    /// Answer requests on the loopback port until one is the redirect. Other
    /// requests, a browser asking for a favicon or something on this machine
    /// probing the port, are turned away without ending the sign-in.
    async fn redirect(&self) -> Result<Redirect, AuthError> {
        loop {
            let (mut stream, _) = match &self.listener_v6 {
                Some(v6) => tokio::select! {
                    accepted = self.listener.accept() => accepted,
                    accepted = v6.accept() => accepted,
                },
                None => self.listener.accept().await,
            }
            .map_err(AuthError::Loopback)?;

            let Some(target) = request_target(&mut stream).await else {
                respond(
                    &mut stream,
                    "400 Bad Request",
                    "That was not a request Brolga understands.",
                )
                .await;
                continue;
            };

            match self.read_redirect(&target) {
                Some(redirect) => {
                    let message = match &redirect {
                        Redirect::Code(_) => {
                            "You are signed in. You can close this tab and go back to Brolga."
                        }
                        Redirect::Refused { .. } => {
                            "The sign-in did not finish. Go back to Brolga to see why."
                        }
                    };
                    respond(&mut stream, "200 OK", message).await;
                    return Ok(redirect);
                }
                None => respond(&mut stream, "404 Not Found", "There is nothing here.").await,
            }
        }
    }

    /// A redirect is a request for `/` carrying our own `state` back. One
    /// with a different state did not come from the sign-in we started.
    fn read_redirect(&self, target: &str) -> Option<Redirect> {
        let url = Url::parse(&format!("http://localhost{target}")).ok()?;
        if url.path() != "/" {
            return None;
        }

        let get = |name: &str| {
            url.query_pairs()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.into_owned())
        };
        if get("state").as_deref() != Some(self.state.as_str()) {
            return None;
        }

        match (get("code"), get("error")) {
            (Some(code), _) => Some(Redirect::Code(code)),
            (None, Some(error)) => Some(Redirect::Refused {
                error,
                description: get("error_description").unwrap_or_default(),
            }),
            (None, None) => None,
        }
    }
}

/// The path and query of a `GET`, or nothing if this is not one we can read.
async fn request_target(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];

    // Read to the end of the headers, or give up at the limit.
    while !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
        let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk))
            .await
            .ok()?
            .ok()?;
        if read == 0 || buffer.len() + read > MAX_REQUEST_BYTES {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }

    let head = String::from_utf8_lossy(&buffer);
    let mut parts = head.lines().next()?.split(' ');

    (parts.next()? == "GET").then(|| parts.next().map(str::to_owned))?
}

/// A failed write only means the browser has already gone.
async fn respond(stream: &mut TcpStream, status: &str, message: &str) {
    let body = format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>Brolga</title>\
         <body style=\"font-family:system-ui,sans-serif;margin:4rem auto;max-width:32rem;text-align:center\">\
         <p>{message}</p></body></html>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );

    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// The numeric codes in a description such as `AADSTS65004: User declined…`.
/// An authorize redirect carries them only in its text.
fn codes_in(description: &str) -> Vec<u32> {
    description
        .split("AADSTS")
        .skip(1)
        .filter_map(|rest| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_read_out_of_a_description() {
        assert_eq!(
            codes_in("AADSTS65004: User declined to consent. Trace ID: 1234"),
            [65004]
        );
        assert_eq!(
            codes_in("AADSTS50076: one. AADSTS53003: two."),
            [50076, 53003]
        );
        assert_eq!(codes_in("nothing to see"), Vec::<u32>::new());
    }
}
