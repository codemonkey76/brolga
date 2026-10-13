use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use brolga_commands::Registry;
use interprocess::local_socket::ListenerOptions;
use interprocess::local_socket::tokio::{Listener, Stream, prelude::*};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::proof;
use crate::protocol::{Call, ErrorBody, MAX_LINE_BYTES, Outcome, Request, Response};
use crate::{Client, Endpoint};

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("another copy of the client is already listening on the control channel")]
    AlreadyRunning,
    /// The channel's name is held by something that could not show it is
    /// this user's client: on a shared machine, another user's program.
    #[error(
        "something that is not a Brolga started by this user is answering on the control channel"
    )]
    Taken,
    #[error("the control channel could not be opened: {0}")]
    Io(#[from] io::Error),
}

/// How long a copy that is starting is given to write its key.
const STARTING: std::time::Duration = std::time::Duration::from_millis(250);

/// The control channel, bound and ready to answer.
pub struct Server {
    listener: Listener,
    /// What a caller is shown this client by, and the file it is kept in.
    key: Option<(Arc<proof::Key>, PathBuf)>,
}

impl Server {
    /// Start listening. A socket file left behind by a client that crashed is
    /// replaced; one that something still answers on is another running client.
    ///
    /// A pipe is made for its owner alone, and a key is kept for callers to
    /// hold this client to. Something already answering on the pipe's name
    /// that cannot answer to the key before it is not another running
    /// client, and is [`ServerError::Taken`].
    pub async fn bind(endpoint: &Endpoint) -> Result<Self, ServerError> {
        Self::bind_proving(endpoint, endpoint.key_file()?.as_deref()).await
    }

    pub(crate) async fn bind_proving(
        endpoint: &Endpoint,
        key_file: Option<&Path>,
    ) -> Result<Self, ServerError> {
        // A copy that is just starting has the name before it has written
        // its key, so what looks taken is looked at once more, a moment on.
        let mut again = key_file.is_some();
        let listener = loop {
            let found = match Client::connect_proving(endpoint, key_file).await {
                Ok(_) => return Err(ServerError::AlreadyRunning),
                Err(error)
                    if key_file.is_some() && error.kind() == io::ErrorKind::PermissionDenied =>
                {
                    ServerError::Taken
                }
                // Nothing answers, or there is no key for anything to answer
                // to. Something may still hold the name, and making the
                // listener then fails, since it asks to be the first of its name.
                Err(_) => match options(endpoint)?.create_tokio() {
                    Ok(listener) => break listener,
                    Err(error)
                        if key_file.is_some()
                            && error.kind() == io::ErrorKind::PermissionDenied =>
                    {
                        ServerError::Taken
                    }
                    Err(error) => return Err(ServerError::Io(error)),
                },
            };
            if !again {
                return Err(found);
            }
            again = false;
            tokio::time::sleep(STARTING).await;
        };
        // After the listener, so that a key is only ever that of the client
        // that holds the name.
        let key = match key_file {
            Some(file) => Some((Arc::new(proof::new_key(file)?), file.to_path_buf())),
            None => None,
        };

        Ok(Self { listener, key })
    }

    /// Answer callers with these commands until the task is dropped.
    ///
    /// Binding and running are separate so that a client can find out it is a
    /// second copy, by failing to bind, before it has opened any data.
    pub async fn run(self, registry: Arc<Registry>) {
        loop {
            // A caller that fails to connect is that caller's trouble, not a
            // reason to stop answering the others.
            let Ok(connection) = self.listener.accept().await else {
                continue;
            };
            let registry = registry.clone();
            let key = self.key.as_ref().map(|(key, _)| key.clone());

            tokio::spawn(async move {
                // Likewise a caller that hangs up mid-line.
                let _ = serve(connection, registry, key).await;
            });
        }
    }
}

impl Drop for Server {
    /// The key goes with the client: what is left of it would be no use to
    /// a caller, and is one thing fewer to be read.
    fn drop(&mut self) {
        if let Some((_, file)) = &self.key {
            let _ = std::fs::remove_file(file);
        }
    }
}

#[cfg(not(windows))]
fn options(endpoint: &Endpoint) -> io::Result<ListenerOptions<'_>> {
    Ok(ListenerOptions::new()
        .name(endpoint.name()?)
        .try_overwrite(true))
}

/// The pipe's access list, in the security descriptor string format
/// ("Security Descriptor String Format" and "ACE Strings", SOURCES.md):
/// protected, so nothing is inherited, with one entry that allows all
/// access to the pipe's owner (`OW`, the owner rights identifier) and so
/// none to anyone else.
#[cfg(windows)]
const OWNER_ONLY: &str = "D:P(A;;GA;;;OW)";

#[cfg(windows)]
fn options(endpoint: &Endpoint) -> io::Result<ListenerOptions<'_>> {
    use interprocess::os::windows::local_socket::ListenerOptionsExt;
    use interprocess::os::windows::security_descriptor::SecurityDescriptor;

    let list = widestring::U16CString::from_str(OWNER_ONLY).map_err(io::Error::other)?;

    Ok(ListenerOptions::new()
        .name(endpoint.name()?)
        .security_descriptor(SecurityDescriptor::deserialize(&list)?))
}

async fn serve(
    connection: Stream,
    registry: Arc<Registry>,
    key: Option<Arc<proof::Key>>,
) -> io::Result<()> {
    let (receive, mut send) = connection.split();
    let mut receive = BufReader::new(receive);
    let mut line = String::new();

    loop {
        line.clear();
        let read = (&mut receive)
            .take(MAX_LINE_BYTES + 1)
            .read_line(&mut line)
            .await?;

        if read == 0 {
            return Ok(());
        }
        if !line.ends_with('\n') && read as u64 > MAX_LINE_BYTES {
            let response = refuse(
                Value::Null,
                "too_large",
                "a request must fit on one line of at most 1 MiB",
            );
            write(&mut send, &response).await?;
            return Ok(());
        }
        if line.trim().is_empty() {
            continue;
        }

        let response = answer(&line, &registry, key.as_deref()).await;
        write(&mut send, &response).await?;
    }
}

async fn answer(line: &str, registry: &Registry, key: Option<&proof::Key>) -> Response {
    let request: Request = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(e) => {
            // Give the id back where the line was at least JSON with one in it.
            let id = serde_json::from_str::<Value>(line)
                .ok()
                .and_then(|v| v.get("id").cloned())
                .unwrap_or(Value::Null);
            return refuse(id, "bad_request", &format!("not a request: {e}"));
        }
    };

    let outcome = match request.call {
        Call::ListCommands => match serde_json::to_value(registry.specs().collect::<Vec<_>>()) {
            Ok(specs) => Outcome::Result(specs),
            Err(e) => Outcome::Error(ErrorBody {
                code: "failed".to_owned(),
                message: e.to_string(),
            }),
        },
        Call::Prove { challenge } => match key.and_then(|key| proof::answer(key, &challenge)) {
            Some(proof) => Outcome::Result(serde_json::json!({ "proof": proof })),
            None => Outcome::Error(ErrorBody {
                code: "no_proof".to_owned(),
                message: if key.is_some() {
                    "a challenge is 32 bytes, written as 64 hex digits".to_owned()
                } else {
                    "this channel is kept to its user by where it is, and has no proof to give"
                        .to_owned()
                },
            }),
        },
        Call::Execute { command, args } => match registry.execute(&command, args).await {
            Ok(result) => Outcome::Result(result),
            Err(e) => Outcome::Error(ErrorBody::from(&e)),
        },
    };

    Response {
        id: request.id,
        outcome,
    }
}

fn refuse(id: Value, code: &str, message: &str) -> Response {
    Response {
        id,
        outcome: Outcome::Error(ErrorBody {
            code: code.to_owned(),
            message: message.to_owned(),
        }),
    }
}

async fn write(send: &mut (impl AsyncWriteExt + Unpin), response: &Response) -> io::Result<()> {
    let mut line = serde_json::to_vec(response).map_err(io::Error::other)?;
    line.push(b'\n');
    send.write_all(&line).await?;
    send.flush().await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use brolga_commands::builtin_registry;
    use serde_json::json;

    use super::*;

    /// An endpoint of this test's own, and the file its key would be kept in.
    fn place(name: &str) -> (Endpoint, PathBuf) {
        let name = format!("brolga-proving-{name}-{}", std::process::id());
        let endpoint = if cfg!(windows) {
            Endpoint::Pipe(name.clone())
        } else {
            Endpoint::Path(std::env::temp_dir().join(format!("{name}.sock")))
        };

        (endpoint, std::env::temp_dir().join(format!("{name}.key")))
    }

    #[tokio::test]
    async fn a_caller_with_the_key_is_answered_and_the_key_goes_with_the_server() {
        let (endpoint, key_file) = place("answered");
        let server = Server::bind_proving(&endpoint, Some(&key_file))
            .await
            .unwrap();
        let running = tokio::spawn(server.run(Arc::new(builtin_registry().unwrap())));

        let mut client = Client::connect_proving(&endpoint, Some(&key_file))
            .await
            .unwrap();
        let version = client.execute("app.version", json!({})).await.unwrap();
        assert_eq!(version["name"], "Brolga");
        // A second copy finds the first, and knows it for one.
        assert!(matches!(
            Server::bind_proving(&endpoint, Some(&key_file)).await,
            Err(ServerError::AlreadyRunning)
        ));

        running.abort();
        let _ = running.await;
        assert!(!key_file.exists());
    }

    #[tokio::test]
    async fn what_answers_without_the_key_is_sent_nothing_and_is_not_taken_for_a_second_copy() {
        let (endpoint, key_file) = place("taken");
        // Someone else got to the name first. They answer, and have no key.
        let theirs = Server::bind_proving(&endpoint, None).await.unwrap();
        tokio::spawn(theirs.run(Arc::new(builtin_registry().unwrap())));
        // A key is here from a client of this user's that has since gone.
        proof::new_key(&key_file).unwrap();

        let Err(refused) = Client::connect_proving(&endpoint, Some(&key_file)).await else {
            panic!("a channel that cannot answer to the key was connected to");
        };
        assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
        assert!(matches!(
            Server::bind_proving(&endpoint, Some(&key_file)).await,
            Err(ServerError::Taken)
        ));

        // With no key at all there is no client of this user's, and nothing is connected to.
        std::fs::remove_file(&key_file).unwrap();
        let Err(none) = Client::connect_proving(&endpoint, Some(&key_file)).await else {
            panic!("a channel was connected to with no key to hold it to");
        };
        assert_eq!(none.kind(), io::ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn a_proof_is_not_given_for_a_challenge_of_the_wrong_length_or_where_there_is_no_key() {
        let registry = builtin_registry().unwrap();
        let key = [7; proof::LENGTH];
        let ask = |challenge: &str| {
            json!({ "id": 1, "method": "prove", "params": { "challenge": challenge } }).to_string()
        };

        let good = answer(&ask(&"ab".repeat(32)), &registry, Some(&key)).await;
        assert!(
            matches!(good.outcome, Outcome::Result(ref r) if r["proof"].as_str().is_some_and(|p| p.len() == 64)),
            "{good:?}"
        );
        for (line, key) in [(ask("abcd"), Some(&key)), (ask(&"ab".repeat(32)), None)] {
            let refused = answer(&line, &registry, key).await;
            assert!(
                matches!(refused.outcome, Outcome::Error(ref e) if e.code == "no_proof"),
                "{refused:?}"
            );
        }
    }
}
