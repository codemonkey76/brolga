#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use brolga_commands::builtin_registry;
use brolga_control::{Client, ClientError, Endpoint, Server, ServerError};
use serde_json::{Value, json};

/// An endpoint of this test's own, so tests can run side by side.
fn endpoint(name: &str) -> Endpoint {
    if cfg!(windows) {
        Endpoint::Pipe(format!("brolga-test-{name}-{}", std::process::id()))
    } else {
        Endpoint::Path(
            std::env::temp_dir().join(format!("brolga-test-{name}-{}.sock", std::process::id())),
        )
    }
}

async fn start(name: &str) -> Endpoint {
    let endpoint = endpoint(name);
    let server = Server::bind(&endpoint).await.unwrap();
    tokio::spawn(server.run(Arc::new(builtin_registry().unwrap())));
    endpoint
}

#[tokio::test]
async fn a_command_runs_over_the_channel() {
    let endpoint = start("run").await;
    let mut client = Client::connect(&endpoint).await.unwrap();

    let version = client.execute("app.version", json!({})).await.unwrap();
    let again = client.execute("app.version", json!({})).await.unwrap();

    assert_eq!(version["name"], "Brolga");
    assert_eq!(again, version);
}

#[tokio::test]
async fn the_channel_lists_every_command() {
    let endpoint = start("list").await;

    let listed = Client::connect(&endpoint)
        .await
        .unwrap()
        .list_commands()
        .await
        .unwrap();

    let local = builtin_registry().unwrap();
    assert_eq!(
        listed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        local.specs().map(|s| s.id.as_str()).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn a_refused_command_comes_back_with_its_code() {
    let endpoint = start("refused").await;
    let mut client = Client::connect(&endpoint).await.unwrap();

    let unknown = client.execute("mail.nothing", json!({})).await.unwrap_err();
    let misfit = client
        .execute("app.version", json!({ "x": 1 }))
        .await
        .unwrap_err();

    assert!(
        matches!(unknown, ClientError::Remote(ref e) if e.code == "unknown_command"),
        "{unknown:?}"
    );
    assert!(
        matches!(misfit, ClientError::Remote(ref e) if e.code == "invalid_arguments"),
        "{misfit:?}"
    );
    // The connection is still good after a refusal.
    assert!(client.execute("app.version", json!({})).await.is_ok());
}

#[tokio::test]
async fn what_is_the_persons_own_to_answer_cannot_be_run_over_the_channel() {
    use brolga_commands::{CommandKind, CommandSpec};

    let mut registry = builtin_registry().unwrap();
    let answered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = answered.clone();
    registry
        .register(
            CommandSpec {
                id: "app.confirmations.answer".to_owned(),
                title: "Answer a question".to_owned(),
                description: "Stands in for the real one, for the test.".to_owned(),
                kind: CommandKind::Action,
                consequential: false,
                mcp_tool: None,
                args_schema: json!({ "type": "object" }),
                result_schema: json!({ "type": "object" }),
                examples: vec![json!({})],
            },
            Arc::new(move |_| {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                Box::pin(async { Ok(json!({ "answered": true })) })
            }),
        )
        .unwrap();
    let endpoint = endpoint("own");
    let server = Server::bind(&endpoint).await.unwrap();
    tokio::spawn(server.run(Arc::new(registry)));
    let mut client = Client::connect(&endpoint).await.unwrap();

    let refused = client
        .execute(
            "app.confirmations.answer",
            json!({ "id": 1, "allow": true }),
        )
        .await
        .unwrap_err();

    assert!(
        matches!(refused, ClientError::Remote(ref e) if e.code == "window_only" && e.message.contains("window")),
        "{refused:?}"
    );
    assert!(!answered.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn a_line_that_is_not_a_request_is_answered_not_dropped() {
    use interprocess::local_socket::tokio::{Stream, prelude::*};
    use interprocess::local_socket::{GenericFilePath, GenericNamespaced, ToFsName, ToNsName};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let endpoint = start("garbage").await;
    let name = match &endpoint {
        Endpoint::Path(p) => p.as_path().to_fs_name::<GenericFilePath>().unwrap(),
        Endpoint::Pipe(n) => n.as_str().to_ns_name::<GenericNamespaced>().unwrap(),
    };
    let (receive, mut send) = Stream::connect(name).await.unwrap().split();
    let mut receive = BufReader::new(receive);

    send.write_all(b"this is not json\n{\"id\":9,\"method\":\"no_such_method\"}\n")
        .await
        .unwrap();

    let mut first = String::new();
    receive.read_line(&mut first).await.unwrap();
    let mut second = String::new();
    receive.read_line(&mut second).await.unwrap();

    let first: Value = serde_json::from_str(&first).unwrap();
    let second: Value = serde_json::from_str(&second).unwrap();
    assert_eq!(first["error"]["code"], "bad_request");
    assert_eq!(first["id"], Value::Null);
    assert_eq!(second["error"]["code"], "bad_request");
    assert_eq!(second["id"], 9);
}

#[tokio::test]
async fn a_second_server_on_the_same_endpoint_is_refused() {
    let endpoint = start("twice").await;

    let second = Server::bind(&endpoint).await;

    assert!(matches!(second, Err(ServerError::AlreadyRunning)));
}

#[cfg(unix)]
#[tokio::test]
async fn a_socket_file_left_by_a_crash_is_replaced() {
    let endpoint = endpoint("stale");
    let Endpoint::Path(path) = &endpoint else {
        unreachable!()
    };
    // What a killed process leaves: a socket file nothing listens on.
    drop(std::os::unix::net::UnixListener::bind(path).unwrap());
    assert!(path.exists());

    let server = Server::bind(&endpoint).await.unwrap();
    tokio::spawn(server.run(Arc::new(builtin_registry().unwrap())));

    assert!(
        Client::connect(&endpoint)
            .await
            .unwrap()
            .execute("app.version", json!({}))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn connecting_with_nothing_listening_is_an_error() {
    assert!(Client::connect(&endpoint("nobody")).await.is_err());
}
