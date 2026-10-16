#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Outside tool servers through the commands (docs/spec-mail-instructions.md,
//! "Outside tools", stage 4a).

use std::sync::Arc;

use brolga_auth::{Authority, MemorySecrets};
use brolga_commands::Registry;
use brolga_core::{Core, CoreOptions};
use brolga_fake_graph::{FakeGraph, FakeIdentity};
use brolga_fake_mcp::FakeMcp;
use serde_json::{Value, json};

async fn core(identity: &FakeIdentity, graph: &FakeGraph) -> Arc<Registry> {
    Core::open(CoreOptions {
        data_dir: None,
        downloads_dir: None,
        secrets: Arc::new(MemorySecrets::default()),
        // A browser that follows any sign-in page to where it sends it back.
        open_browser: Arc::new(|url| {
            let url = url.to_owned();
            tokio::spawn(async move {
                let _ = reqwest::get(url).await;
            });
            Ok(())
        }),
        authority: Authority::new(identity.base_url()),
        graph_base: Some(graph.base_url()),
        default_client_id: Some("client".to_owned()),
        fetch_images_from_private_hosts: false,
        host_name: None,
        anthropic_base: None,
    })
    .unwrap()
    .registry()
}

async fn run(on: &Registry, command: &str, args: Value) -> Value {
    on.execute_from_window(command, args)
        .await
        .unwrap_or_else(|e| panic!("{command}: {e}"))
}

#[tokio::test]
async fn a_server_is_added_signed_in_to_tried_and_removed_and_only_the_person_can_add_one() {
    let identity = FakeIdentity::start().await.unwrap();
    let graph = FakeGraph::start().await.unwrap();
    let mcp = FakeMcp::start().await.unwrap();
    let registry = core(&identity, &graph).await;

    // Not from outside the window.
    for (command, args) in [
        ("mcp_servers.add", json!({ "name": "x", "url": mcp.url() })),
        (
            "mcp_servers.set_token",
            json!({ "server_id": "x", "token": "t" }),
        ),
        ("mcp_servers.sign_in", json!({ "server_id": "x" })),
    ] {
        assert_eq!(
            registry.execute(command, args).await.unwrap_err().code(),
            "window_only",
            "{command}"
        );
    }
    // Not over plain http to another computer.
    let error = registry
        .execute_from_window(
            "mcp_servers.add",
            json!({ "name": "Helpdesk", "url": "http://helpdesk.example.com/mcp" }),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("https"), "{error}");

    let added = run(
        &registry,
        "mcp_servers.add",
        json!({ "name": "Helpdesk", "url": mcp.url() }),
    )
    .await;
    let id = added["server"]["id"].as_str().unwrap().to_owned();
    assert_eq!(added["server"]["signed_in"], Value::Null);
    let tried = run(&registry, "mcp_servers.test", json!({ "server_id": id })).await;
    assert_eq!(tried["ok"], false);
    assert!(tried["error"].as_str().unwrap().contains("not signed in"));

    // Through the browser.
    let signed = run(&registry, "mcp_servers.sign_in", json!({ "server_id": id })).await;
    assert_eq!(signed["server"]["signed_in"], "browser");
    assert_eq!(signed["server"]["tools"], 2);
    let tools = run(&registry, "mcp_servers.tools", json!({ "server_id": id })).await;
    assert_eq!(tools["tools"][0]["name"], "create_ticket");
    assert!(
        !run(&registry, "mcp_servers.list", json!({}))
            .await
            .to_string()
            .contains("at-"),
        "no token is given out"
    );

    // With a token instead.
    mcp.accept_token("pat-1");
    let with_token = run(
        &registry,
        "mcp_servers.set_token",
        json!({ "server_id": id, "token": "pat-1" }),
    )
    .await;
    assert_eq!(with_token["server"]["signed_in"], "token");
    assert_eq!(
        run(&registry, "mcp_servers.test", json!({ "server_id": id })).await["tools"],
        2
    );

    assert_eq!(
        run(&registry, "mcp_servers.remove", json!({ "server_id": id })).await["removed"],
        true
    );
    assert_eq!(
        run(&registry, "mcp_servers.list", json!({})).await["servers"],
        json!([])
    );
}
