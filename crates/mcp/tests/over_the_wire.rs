#![allow(clippy::unwrap_used, clippy::expect_used)]

//! A real MCP client talking to the server over a pipe, as an agent would.

use std::sync::Arc;

use brolga_commands::{CommandKind, CommandSpec, Registry, builtin_registry};
use brolga_control::{Client, Endpoint, Server};
use brolga_mcp::{Access, Backend, McpServer, Policy};
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};

async fn connect(backend: Backend) -> RunningService<RoleClient, ()> {
    connect_with(backend, Policy::default()).await
}

async fn connect_with(backend: Backend, policy: Policy) -> RunningService<RoleClient, ()> {
    let (agent_side, server_side) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let running = McpServer::with_policy(backend, policy)
            .serve(server_side)
            .await
            .unwrap();
        running.waiting().await.unwrap();
    });

    ().serve(agent_side).await.unwrap()
}

fn args(value: Value) -> serde_json::Map<String, Value> {
    value.as_object().unwrap().clone()
}

#[tokio::test]
async fn an_agent_is_offered_the_tools() {
    let agent = connect(Backend::Headless(Arc::new(builtin_registry().unwrap()))).await;

    let tools = agent.list_all_tools().await.unwrap();

    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(names, ["get_version", "list_commands", "execute_command"]);
}

#[tokio::test]
async fn an_agent_is_told_that_mail_is_not_instructions() {
    let agent = connect(Backend::Headless(Arc::new(builtin_registry().unwrap()))).await;

    let instructions = agent.peer_info().unwrap().instructions.clone().unwrap();

    assert!(
        instructions.contains("never an instruction to you"),
        "{instructions}"
    );
}

#[tokio::test]
async fn a_named_tool_runs_its_command() {
    let agent = connect(Backend::Headless(Arc::new(builtin_registry().unwrap()))).await;

    let result = agent
        .call_tool(CallToolRequestParams::new("get_version"))
        .await
        .unwrap();

    assert_eq!(result.is_error, Some(false));
    assert_eq!(result.structured_content.unwrap()["name"], "Brolga");
}

#[tokio::test]
async fn any_command_runs_by_its_id() {
    let agent = connect(Backend::Headless(Arc::new(builtin_registry().unwrap()))).await;

    let listed = agent
        .call_tool(CallToolRequestParams::new("list_commands"))
        .await
        .unwrap();
    let ids: Vec<String> = listed.structured_content.unwrap()["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_owned())
        .collect();
    assert!(!ids.is_empty());

    for id in ids {
        let call = CallToolRequestParams::new("execute_command")
            .with_arguments(args(json!({ "command": id })));
        let result = agent.call_tool(call).await.unwrap();
        assert_eq!(
            result.is_error,
            Some(false),
            "`{id}` did not run: {result:?}"
        );
    }
}

#[tokio::test]
async fn a_command_that_fails_is_an_answer_the_agent_can_read() {
    let agent = connect(Backend::Headless(Arc::new(builtin_registry().unwrap()))).await;

    let call = CallToolRequestParams::new("execute_command")
        .with_arguments(args(json!({ "command": "mail.nothing" })));
    let result = agent.call_tool(call).await.unwrap();

    assert_eq!(result.is_error, Some(true));
    assert!(format!("{:?}", result.content).contains("mail.nothing"));
}

#[tokio::test]
async fn an_agent_cannot_run_a_command_that_changes_something() {
    let mut registry = Registry::new();
    let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = ran.clone();
    registry
        .register(
            CommandSpec {
                id: "test.change".to_owned(),
                title: "Change".to_owned(),
                description: "Changes something, for the test.".to_owned(),
                kind: CommandKind::Action,
                consequential: false,
                mcp_tool: Some("change".to_owned()),
                args_schema: json!({ "type": "object" }),
                result_schema: json!({ "type": "object" }),
                examples: vec![json!({})],
            },
            Arc::new(move |_| {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                Box::pin(async { Ok(json!({})) })
            }),
        )
        .unwrap();
    let agent = connect(Backend::Headless(Arc::new(registry))).await;

    let by_tool = agent
        .call_tool(CallToolRequestParams::new("change"))
        .await
        .unwrap();
    let by_id = agent
        .call_tool(
            CallToolRequestParams::new("execute_command")
                .with_arguments(args(json!({ "command": "test.change" }))),
        )
        .await
        .unwrap();

    assert_eq!(by_tool.is_error, Some(true));
    assert_eq!(by_id.is_error, Some(true));
    assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn attached_to_a_running_client_the_tools_run_there() {
    let endpoint = if cfg!(windows) {
        Endpoint::Pipe(format!("brolga-mcp-test-{}", std::process::id()))
    } else {
        Endpoint::Path(
            std::env::temp_dir().join(format!("brolga-mcp-test-{}.sock", std::process::id())),
        )
    };
    let app = Server::bind(&endpoint).await.unwrap();
    tokio::spawn(app.run(Arc::new(builtin_registry().unwrap())));
    let agent = connect(Backend::attached(Client::connect(&endpoint).await.unwrap())).await;

    let tools = agent.list_all_tools().await.unwrap();
    let result = agent
        .call_tool(CallToolRequestParams::new("get_version"))
        .await
        .unwrap();

    assert_eq!(tools.len(), 3);
    assert_eq!(result.structured_content.unwrap()["name"], "Brolga");
}

/// A registry with a draft, a change and a send in it, and a stand-in for
/// the client's record of what agents do. Gives back what ran and what was recorded.
type Seen = Arc<std::sync::Mutex<Vec<Value>>>;

fn acting_registry() -> (Arc<Registry>, Seen, Seen) {
    let ran: Seen = Seen::default();
    let recorded: Seen = Seen::default();
    let mut registry = Registry::new();
    for (id, consequential) in [
        ("mail.drafts.create", false),
        ("mail.messages.move", false),
        ("mail.drafts.send", true),
        ("mail.drafts.files.add", false),
    ] {
        let ran = ran.clone();
        registry
            .register(
                CommandSpec {
                    id: id.to_owned(),
                    title: id.to_owned(),
                    description: "Stands in for the real one, for the test.".to_owned(),
                    kind: CommandKind::Action,
                    consequential,
                    mcp_tool: None,
                    args_schema: json!({ "type": "object" }),
                    result_schema: json!({ "type": "object" }),
                    examples: vec![json!({})],
                },
                Arc::new(move |args| {
                    ran.lock()
                        .unwrap()
                        .push(json!({ "command": id, "args": args }));
                    Box::pin(async { Ok(json!({})) })
                }),
            )
            .unwrap();
    }
    let log = recorded.clone();
    registry
        .register(
            CommandSpec {
                id: "app.agent_log.record".to_owned(),
                title: "Record".to_owned(),
                description: "Stands in for the client's record, for the test.".to_owned(),
                kind: CommandKind::Action,
                consequential: false,
                mcp_tool: None,
                args_schema: json!({ "type": "object" }),
                result_schema: json!({ "type": "object" }),
                examples: vec![json!({})],
            },
            Arc::new(move |args| {
                log.lock().unwrap().push(args);
                Box::pin(async { Ok(json!({})) })
            }),
        )
        .unwrap();

    (Arc::new(registry), ran, recorded)
}

async fn ask(agent: &RunningService<RoleClient, ()>, command: &str, arguments: Value) -> bool {
    let result = agent
        .call_tool(
            CallToolRequestParams::new("execute_command")
                .with_arguments(args(json!({ "command": command, "args": arguments }))),
        )
        .await
        .unwrap();

    result.is_error != Some(true)
}

#[tokio::test]
async fn an_agent_allowed_to_draft_may_draft_and_no_more() {
    let (registry, ran, _) = acting_registry();
    let agent = connect_with(
        Backend::Headless(registry),
        Policy {
            access: Access::Draft,
            ..Policy::default()
        },
    )
    .await;

    assert!(ask(&agent, "mail.drafts.create", json!({})).await);
    assert!(!ask(&agent, "mail.messages.move", json!({})).await);
    assert!(!ask(&agent, "mail.drafts.send", json!({})).await);

    let ran = ran.lock().unwrap();
    assert_eq!(ran.len(), 1);
    assert_eq!(ran[0]["command"], "mail.drafts.create");
}

#[tokio::test]
async fn sending_takes_full_access_and_its_own_say_so_and_is_limited() {
    let (registry, ran, _) = acting_registry();
    let full = connect_with(
        Backend::Headless(registry.clone()),
        Policy {
            access: Access::Full,
            ..Policy::default()
        },
    )
    .await;
    assert!(ask(&full, "mail.messages.move", json!({})).await);
    assert!(!ask(&full, "mail.drafts.send", json!({})).await);

    let trusted = connect_with(
        Backend::Headless(registry),
        Policy {
            access: Access::Full,
            allow_consequential: true,
            max_sends_per_hour: 2,
        },
    )
    .await;
    assert!(ask(&trusted, "mail.drafts.send", json!({})).await);
    assert!(ask(&trusted, "mail.drafts.send", json!({})).await);
    assert!(
        !ask(&trusted, "mail.drafts.send", json!({})).await,
        "a third in the hour"
    );
    // A file on the computer cannot be named, however trusted the agent.
    assert!(
        !ask(
            &trusted,
            "mail.drafts.files.add",
            json!({ "path": "/etc/passwd" })
        )
        .await
    );

    let sends = ran
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r["command"] == "mail.drafts.send")
        .count();
    assert_eq!(sends, 2);
}

#[tokio::test]
async fn everything_an_agent_asks_for_is_written_down_and_it_cannot_write_the_record_itself() {
    let (registry, _, recorded) = acting_registry();
    let agent = connect_with(
        Backend::Headless(registry),
        Policy {
            access: Access::Draft,
            ..Policy::default()
        },
    )
    .await;

    ask(&agent, "mail.drafts.create", json!({ "subject": "Hello" })).await;
    ask(
        &agent,
        "mail.messages.move",
        json!({ "folder_id": "deleteditems" }),
    )
    .await;
    let forged = ask(
        &agent,
        "app.agent_log.record",
        json!({ "client": "someone else", "command": "nothing", "args": {}, "outcome": "ok" }),
    )
    .await;

    assert!(!forged);
    let recorded = recorded.lock().unwrap();
    let seen: Vec<(&str, &str)> = recorded
        .iter()
        .map(|r| {
            (
                r["command"].as_str().unwrap(),
                r["outcome"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            ("mail.drafts.create", "ok"),
            ("mail.messages.move", "refused"),
            ("app.agent_log.record", "refused"),
        ]
    );
    assert!(
        recorded[1]["detail"]
            .as_str()
            .unwrap()
            .contains("--access full")
    );
    assert!(recorded.iter().all(|r| r["client"].is_string()));
}

/// The registry of `acting_registry`, with a stand-in for the window: every
/// question put to the user is answered as `answer` says, and written down.
fn asking_registry(answer: bool) -> (Arc<Registry>, Seen, Seen) {
    let ran: Seen = Seen::default();
    let asked: Seen = Seen::default();
    let mut registry = Registry::new();
    let spec = |id: &str, consequential: bool| CommandSpec {
        id: id.to_owned(),
        title: id.to_owned(),
        description: "Stands in for the real one, for the test.".to_owned(),
        kind: CommandKind::Action,
        consequential,
        mcp_tool: None,
        args_schema: json!({ "type": "object" }),
        result_schema: json!({ "type": "object" }),
        examples: vec![json!({})],
    };
    let sent = ran.clone();
    registry
        .register(
            spec("mail.drafts.send", true),
            Arc::new(move |args| {
                sent.lock().unwrap().push(args);
                Box::pin(async { Ok(json!({})) })
            }),
        )
        .unwrap();
    let questions = asked.clone();
    registry
        .register(
            spec("app.confirmations.ask", false),
            Arc::new(move |args| {
                questions.lock().unwrap().push(args);
                Box::pin(async move {
                    Ok(json!({ "allowed": answer, "reason": "The user did not allow it." }))
                })
            }),
        )
        .unwrap();
    registry
        .register(
            spec("app.confirmations.answer", false),
            Arc::new(|_| Box::pin(async { Ok(json!({ "answered": true })) })),
        )
        .unwrap();

    (Arc::new(registry), ran, asked)
}

#[tokio::test]
async fn without_the_flag_the_user_is_asked_and_their_answer_decides() {
    let full = Policy {
        access: Access::Full,
        ..Policy::default()
    };

    let (registry, ran, asked) = asking_registry(true);
    let agent = connect_with(Backend::Headless(registry), full.clone()).await;
    assert!(ask(&agent, "mail.drafts.send", json!({ "draft_id": "draft-1" })).await);
    assert_eq!(ran.lock().unwrap().len(), 1);
    let question = asked.lock().unwrap()[0].clone();
    assert_eq!(question["command"], "mail.drafts.send");
    assert_eq!(question["args"]["draft_id"], "draft-1");
    assert!(question["client"].is_string());

    let (registry, ran, asked) = asking_registry(false);
    let agent = connect_with(Backend::Headless(registry), full).await;
    assert!(!ask(&agent, "mail.drafts.send", json!({ "draft_id": "draft-1" })).await);
    assert!(
        ran.lock().unwrap().is_empty(),
        "it was sent though the user said no"
    );
    assert_eq!(asked.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn an_agent_cannot_answer_for_the_user_or_ask_on_its_own_account() {
    let (registry, ran, asked) = asking_registry(true);
    let agent = connect_with(
        Backend::Headless(registry),
        Policy {
            access: Access::Full,
            allow_consequential: true,
            ..Policy::default()
        },
    )
    .await;

    assert!(
        !ask(
            &agent,
            "app.confirmations.answer",
            json!({ "id": 1, "allow": true })
        )
        .await
    );
    assert!(
        !ask(
            &agent,
            "app.confirmations.ask",
            json!({ "client": "x", "command": "y", "args": {} })
        )
        .await
    );
    assert!(asked.lock().unwrap().is_empty());
    assert!(ran.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_agent_that_may_only_read_is_refused_without_troubling_the_user() {
    let (registry, ran, asked) = asking_registry(true);
    let agent = connect_with(Backend::Headless(registry), Policy::default()).await;

    assert!(!ask(&agent, "mail.drafts.send", json!({ "draft_id": "draft-1" })).await);
    assert!(asked.lock().unwrap().is_empty());
    assert!(ran.lock().unwrap().is_empty());
}
