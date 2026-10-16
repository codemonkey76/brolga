#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Signing in to an MCP server and reading its tools, against a stand-in
//! that holds requests to what the MCP specification asks.

use std::sync::Arc;
use std::time::Duration;

use brolga_auth::{MemorySecrets, SecretStore};
use brolga_fake_mcp::FakeMcp;
use brolga_tools::{Server, SignedIn, ToolsError};

fn server(fake: &FakeMcp) -> Server {
    Server {
        id: "s1".to_owned(),
        name: "Helpdesk".to_owned(),
        url: fake.url(),
    }
}

/// A browser that follows the sign-in page to where it sends it back.
fn browser(url: &str) -> std::io::Result<()> {
    let url = url.to_owned();
    tokio::spawn(async move {
        let _ = reqwest::get(url).await;
    });
    Ok(())
}

#[tokio::test]
async fn a_server_is_reached_with_an_access_token() {
    let fake = FakeMcp::start().await.unwrap();
    fake.accept_token("pat-123");
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecrets::default());
    let server = server(&fake);

    assert!(matches!(
        brolga_tools::tools(secrets.clone(), &server).await,
        Err(ToolsError::NotSignedIn(_))
    ));
    brolga_tools::save_token(secrets.as_ref(), &server, "Bearer pat-123").unwrap();
    assert_eq!(
        brolga_tools::signed_in(secrets.as_ref(), &server),
        Some(SignedIn::Token)
    );
    let tools = brolga_tools::tools(secrets.clone(), &server).await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["create_ticket", "list_tickets"]);
    assert_eq!(tools[0].input_schema["required"][0], "client_id");

    // A token that is no longer taken is not taken.
    brolga_tools::save_token(secrets.as_ref(), &server, "pat-wrong").unwrap();
    assert!(brolga_tools::tools(secrets.clone(), &server).await.is_err());
}

#[tokio::test]
async fn signing_in_through_the_browser_registers_brolga_and_keeps_and_refreshes_the_token() {
    let fake = FakeMcp::start().await.unwrap();
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecrets::default());
    let server = server(&fake);

    brolga_tools::sign_in(secrets.clone(), &server, browser, Duration::from_secs(10))
        .await
        .unwrap();
    assert_eq!(fake.registered(), 1);
    assert_eq!(
        brolga_tools::signed_in(secrets.as_ref(), &server),
        Some(SignedIn::Browser)
    );
    assert_eq!(
        brolga_tools::tools(secrets.clone(), &server)
            .await
            .unwrap()
            .len(),
        2
    );

    // Kept: a later connection needs no browser.
    assert_eq!(
        brolga_tools::tools(secrets.clone(), &server)
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(fake.registered(), 1);

    // A token that has run out is refreshed, with no browser.
    fake.short_lived_tokens();
    brolga_tools::forget(secrets.as_ref(), &server).unwrap();
    brolga_tools::sign_in(secrets.clone(), &server, browser, Duration::from_secs(10))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        brolga_tools::tools(secrets.clone(), &server)
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(fake.refreshed() >= 1);

    brolga_tools::forget(secrets.as_ref(), &server).unwrap();
    assert_eq!(brolga_tools::signed_in(secrets.as_ref(), &server), None);
}

#[tokio::test]
async fn a_tool_is_called_with_its_arguments() {
    let fake = FakeMcp::start().await.unwrap();
    fake.accept_token("pat-1");
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecrets::default());
    let server = server(&fake);
    brolga_tools::save_token(secrets.as_ref(), &server, "pat-1").unwrap();

    let arguments =
        serde_json::json!({ "client_id": 15, "subject": "Phones", "description": "Quote" });
    let called = brolga_tools::call(
        secrets,
        &server,
        "create_ticket",
        arguments.as_object().unwrap().clone(),
    )
    .await
    .unwrap();
    assert!(called.ok);
    assert_eq!(called.text, "create_ticket done: TKT-01234");
    assert_eq!(fake.calls(), [("create_ticket".to_owned(), arguments)]);
}
