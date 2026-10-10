use std::io;

use brolga_commands::CommandSpec;
use interprocess::local_socket::tokio::{RecvHalf, SendHalf, Stream, prelude::*};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::Endpoint;
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

/// One connection to a running client.
pub struct Client {
    receive: BufReader<RecvHalf>,
    send: SendHalf,
    next_id: u64,
}

impl Client {
    /// Connect to a running client. An error here usually means none is running.
    pub async fn connect(endpoint: &Endpoint) -> io::Result<Self> {
        let (receive, send) = Stream::connect(endpoint.name()?).await?.split();

        Ok(Self {
            receive: BufReader::new(receive),
            send,
            next_id: 1,
        })
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
