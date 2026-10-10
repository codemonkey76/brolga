use std::io;
use std::sync::Arc;

use brolga_commands::Registry;
use interprocess::local_socket::ListenerOptions;
use interprocess::local_socket::tokio::{Listener, Stream, prelude::*};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::Endpoint;
use crate::protocol::{Call, ErrorBody, MAX_LINE_BYTES, Outcome, Request, Response};

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("another copy of the client is already listening on the control channel")]
    AlreadyRunning,
    #[error("the control channel could not be opened: {0}")]
    Io(#[from] io::Error),
}

/// The control channel, bound and ready to answer.
pub struct Server {
    listener: Listener,
}

impl Server {
    /// Start listening. A socket file left behind by a client that crashed is
    /// replaced; one that something still answers on is another running client.
    pub async fn bind(endpoint: &Endpoint) -> Result<Self, ServerError> {
        if Stream::connect(endpoint.name()?).await.is_ok() {
            return Err(ServerError::AlreadyRunning);
        }

        let listener = ListenerOptions::new()
            .name(endpoint.name()?)
            .try_overwrite(true)
            .create_tokio()?;

        Ok(Self { listener })
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

            tokio::spawn(async move {
                // Likewise a caller that hangs up mid-line.
                let _ = serve(connection, registry).await;
            });
        }
    }
}

async fn serve(connection: Stream, registry: Arc<Registry>) -> io::Result<()> {
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

        let response = answer(&line, &registry).await;
        write(&mut send, &response).await?;
    }
}

async fn answer(line: &str, registry: &Registry) -> Response {
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
