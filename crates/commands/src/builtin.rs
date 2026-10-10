use std::sync::Arc;

use serde_json::{Value, json};

use crate::{CommandKind, CommandSpec, Handler, Registry, RegistryError};

/// The commands the client has before any account is signed in.
///
/// There is no sign-in yet, so the two that report on accounts have nothing to
/// report and say so. They are here so that every way of running a command has
/// real ones to run.
pub fn builtin_registry() -> Result<Registry, RegistryError> {
    let mut registry = Registry::new();

    registry.register(
        CommandSpec {
            id: "app.version".to_owned(),
            title: "Version".to_owned(),
            description:
                "The name and version of this mail client. Use it to check the client is reachable."
                    .to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: Some("get_version".to_owned()),
            args_schema: no_arguments(),
            result_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "version": { "type": "string" }
                },
                "required": ["name", "version"],
                "additionalProperties": false
            }),
            examples: vec![json!({})],
        },
        handler(|| json!({ "name": "Brolga", "version": env!("CARGO_PKG_VERSION") })),
    )?;

    registry.register(
        CommandSpec {
            id: "app.accounts.list".to_owned(),
            title: "List accounts".to_owned(),
            description: "The mail accounts signed in to this client. Signing in is not built yet, so the list is always empty.".to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: Some("list_accounts".to_owned()),
            args_schema: no_arguments(),
            result_schema: json!({
                "type": "object",
                "properties": {
                    "accounts": { "type": "array", "items": { "type": "object" } }
                },
                "required": ["accounts"],
                "additionalProperties": false
            }),
            examples: vec![json!({})],
        },
        handler(|| json!({ "accounts": [] })),
    )?;

    registry.register(
        CommandSpec {
            id: "app.sync.status".to_owned(),
            title: "Sync status".to_owned(),
            description: "Whether each account is up to date with the server. Syncing is not built yet, so no account is reported.".to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: Some("get_sync_status".to_owned()),
            args_schema: no_arguments(),
            result_schema: json!({
                "type": "object",
                "properties": {
                    "accounts": { "type": "array", "items": { "type": "object" } }
                },
                "required": ["accounts"],
                "additionalProperties": false
            }),
            examples: vec![json!({})],
        },
        handler(|| json!({ "accounts": [] })),
    )?;

    Ok(registry)
}

fn no_arguments() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

fn handler(result: impl Fn() -> Value + Send + Sync + 'static) -> Handler {
    Arc::new(move |_| {
        let value = result();
        Box::pin(async move { Ok(value) })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate the spec asks for: a command with no schema worth the name, no
    /// description or no example that runs does not get into the client.
    #[tokio::test]
    async fn every_command_is_described_and_its_examples_run() {
        let registry = builtin_registry().unwrap();
        let mut seen = 0;

        for spec in registry.specs() {
            seen += 1;
            assert!(!spec.title.trim().is_empty(), "`{}` has no title", spec.id);
            assert!(
                spec.description.len() >= 20,
                "`{}` is not described for an agent",
                spec.id
            );
            assert_eq!(
                spec.args_schema["type"], "object",
                "`{}` must take an object",
                spec.id
            );
            assert!(
                !spec.examples.is_empty(),
                "`{}` has no example, so nothing tests it",
                spec.id
            );

            for example in &spec.examples {
                let result = registry.execute(&spec.id, example.clone()).await;
                assert!(
                    result.is_ok(),
                    "`{}` failed on its own example {example}: {result:?}",
                    spec.id
                );
            }
        }

        assert!(seen > 0, "the client has no commands at all");
    }

    #[tokio::test]
    async fn the_version_is_the_one_the_crate_was_built_as() {
        let result = builtin_registry()
            .unwrap()
            .execute("app.version", json!({}))
            .await
            .unwrap();

        assert_eq!(
            result,
            json!({ "name": "Brolga", "version": env!("CARGO_PKG_VERSION") })
        );
    }

    #[tokio::test]
    async fn a_command_that_takes_nothing_refuses_stray_arguments() {
        let error = builtin_registry()
            .unwrap()
            .execute("app.version", json!({ "x": 1 }))
            .await
            .unwrap_err();

        assert_eq!(error.code(), "invalid_arguments");
    }
}
