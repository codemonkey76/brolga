use std::io;
use std::path::Path;

use brolga_commands::CommandSpec;
use interprocess::local_socket::tokio::{RecvHalf, SendHalf, Stream, prelude::*};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::Endpoint;
use crate::proof;
use crate::protocol::{Call, ErrorBody, MAX_LINE_BYTES, Outcome, Request, Response};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The running client refused or failed the command.
    #[error("{}", .0.message)]
    Remote(ErrorBody),
    #[error("the control channel failed: {0}")]
    Io(#[from] io::Error),
    #[error("the running client answered with something that is not an answer: {0}")]
    Protocol(String),
}

/// How long what answers is given to show it is this user's client. A copy
/// that is still starting answers once it has opened its data, which on a
/// large mailbox is not at once.
const PROOF_WITHIN: std::time::Duration = std::time::Duration::from_secs(30);

/// One connection to a running client.
pub struct Client {
    receive: BufReader<RecvHalf>,
    send: SendHalf,
    next_id: u64,
}

impl Client {
    /// Connect to a running client. An error here usually means none is running.
    ///
    /// Where the channel is a pipe, what answers must show that it is this
    /// user's client before anything else is sent to it. One that cannot is
    /// refused as `PermissionDenied`.
    pub async fn connect(endpoint: &Endpoint) -> io::Result<Self> {
        Self::connect_proving(endpoint, endpoint.key_file()?.as_deref()).await
    }

    /// Connect, and where there is a key file hold what answers to it.
    pub(crate) async fn connect_proving(
        endpoint: &Endpoint,
        key_file: Option<&Path>,
    ) -> io::Result<Self> {
        // Read first: with no key there is no client of this user's to call.
        let key = key_file.map(proof::read_key).transpose()?;
        let (receive, send) = Stream::connect(endpoint.name()?).await?.split();
        let mut client = Self {
            receive: BufReader::new(receive),
            send,
            next_id: 1,
        };
        if let Some(key) = key {
            client.prove(&key).await?;
        }

        Ok(client)
    }

    async fn prove(&mut self, key: &proof::Key) -> io::Result<()> {
        let challenge = proof::challenge()?;
        let expected = proof::answer(key, &challenge);
        let asked = tokio::time::timeout(PROOF_WITHIN, self.call(Call::Prove { challenge }));
        let given = match asked.await {
            Ok(Ok(given)) => given,
            Ok(Err(ClientError::Io(error))) => return Err(error),
            // It answered, and not with a proof.
            Ok(Err(_)) => Value::Null,
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "what holds the control channel did not answer, so nothing more was sent to it",
                ));
            }
        };

        if expected.is_some() && given.get("proof").and_then(Value::as_str) == expected.as_deref() {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "what answers on the control channel is not a Brolga started by this user, so nothing was sent to it",
            ))
        }
    }

    pub async fn list_commands(&mut self) -> Result<Vec<CommandSpec>, ClientError> {
        let specs = self.call(Call::ListCommands).await?;

        serde_json::from_value(specs).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    pub async fn execute(&mut self, command: &str, args: Value) -> Result<Value, ClientError> {
        self.call(Call::Execute {
            command: command.to_owned(),
            args,
        })
        .await
    }

    async fn call(&mut self, call: Call) -> Result<Value, ClientError> {
        let id = Value::from(self.next_id);
        self.next_id += 1;

        let mut line = serde_json::to_vec(&Request {
            id: id.clone(),
            call,
        })
        .map_err(io::Error::other)?;
        line.push(b'\n');
        self.send.write_all(&line).await?;
        self.send.flush().await?;

        let mut answer = String::new();
        let read = (&mut self.receive)
            .take(MAX_LINE_BYTES + 1)
            .read_line(&mut answer)
            .await?;
        if read == 0 {
            return Err(ClientError::Io(io::ErrorKind::UnexpectedEof.into()));
        }

        let response: Response =
            serde_json::from_str(&answer).map_err(|e| ClientError::Protocol(e.to_string()))?;
        match response.outcome {
            // An error may come back without the id, where the line could not be read.
            Outcome::Error(error) => Err(ClientError::Remote(error)),
            Outcome::Result(_) if response.id != id => Err(ClientError::Protocol(
                "the answer is to a different request".to_owned(),
            )),
            Outcome::Result(result) => Ok(result),
        }
    }
}
