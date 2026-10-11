use std::sync::Arc;

use serde_json::{Value, json};

use crate::{CommandKind, CommandSpec, Handler, Registry, RegistryError};

/// The commands that need nothing but the client itself. The core adds the
/// rest, which need its accounts and its data.
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

    /// The same gate is applied to every command the client has in
    /// `crates/core/tests/commands.rs`.
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
