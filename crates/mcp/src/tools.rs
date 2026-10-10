use std::sync::Arc;

use brolga_commands::{CommandKind, CommandSpec};
use rmcp::model::{JsonObject, Tool, ToolAnnotations};
use serde_json::{Value, json};

pub(crate) const LIST_COMMANDS: &str = "list_commands";
pub(crate) const EXECUTE_COMMAND: &str = "execute_command";

/// The tools an agent is offered: one for each command that has a tool name,
/// then the two that reach every command.
pub(crate) fn tools_for(specs: &[CommandSpec]) -> Vec<Tool> {
    let mut tools: Vec<Tool> = specs.iter().filter_map(named_tool).collect();

    tools.push(
        Tool::new(
            LIST_COMMANDS,
            "Every command this mail client has, each with its id, what it does and the JSON Schema of its arguments. \
             Use it to find a command that has no tool of its own, then run that with execute_command.",
            object(json!({ "type": "object", "properties": {}, "additionalProperties": false })),
        )
        .with_annotations(ToolAnnotations::new().read_only(true).open_world(false)),
    );
    tools.push(Tool::new(
        EXECUTE_COMMAND,
        "Run any command of this mail client by its id, with arguments that fit the schema list_commands gives for it.",
        object(json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The command's id, such as app.version." },
                "args": { "type": "object", "description": "The command's arguments. Leave out for a command that takes none." }
            },
            "required": ["command"],
            "additionalProperties": false
        })),
    ));

    tools
}

fn named_tool(spec: &CommandSpec) -> Option<Tool> {
    let name = spec.mcp_tool.clone()?;
    let mut tool = Tool::new(
        name,
        spec.description.clone(),
        object(spec.args_schema.clone()),
    );
    tool.title = Some(spec.title.clone());

    // MCP allows an output schema only where the result is an object.
    if spec.result_schema.get("type") == Some(&json!("object")) {
        tool.output_schema = Some(Arc::new(object(spec.result_schema.clone())));
    }

    Some(
        tool.with_annotations(
            ToolAnnotations::new()
                .read_only(spec.kind != CommandKind::Action)
                .destructive(spec.consequential)
                .open_world(spec.kind == CommandKind::Action),
        ),
    )
}

fn object(value: Value) -> JsonObject {
    match value {
        Value::Object(map) => map,
        _ => JsonObject::new(),
    }
}

/// What a tool call asks for, once the tool's name is looked up.
#[derive(Debug, PartialEq)]
pub(crate) enum Route {
    ListCommands,
    Execute { command: String, args: Value },
}

pub(crate) fn route(
    tool: &str,
    arguments: Option<JsonObject>,
    specs: &[CommandSpec],
) -> Result<Route, String> {
    let arguments = arguments.unwrap_or_default();

    match tool {
        LIST_COMMANDS => Ok(Route::ListCommands),
        EXECUTE_COMMAND => {
            let command = arguments
                .get("command")
                .and_then(Value::as_str)
                .ok_or("execute_command needs `command`, the id of the command to run")?
                .to_owned();
            let args = arguments.get("args").cloned().unwrap_or_else(|| json!({}));

            Ok(Route::Execute { command, args })
        }
        named => specs
            .iter()
            .find(|s| s.mcp_tool.as_deref() == Some(named))
            .map(|s| Route::Execute {
                command: s.id.clone(),
                args: Value::Object(arguments),
            })
            .ok_or_else(|| format!("there is no tool called `{named}`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brolga_commands::builtin_registry;

    fn specs() -> Vec<CommandSpec> {
        builtin_registry().unwrap().specs().cloned().collect()
    }

    #[test]
    fn every_command_is_reachable_by_a_tool() {
        let specs = specs();
        let tools = tools_for(&specs);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();

        // The two that reach everything are always there...
        assert!(names.contains(&LIST_COMMANDS) && names.contains(&EXECUTE_COMMAND));
        for spec in &specs {
            // ...so every command has a path, and each named tool leads to its own command.
            assert_eq!(
                route(
                    EXECUTE_COMMAND,
                    Some(object(json!({ "command": spec.id }))),
                    &specs
                ),
                Ok(Route::Execute {
                    command: spec.id.clone(),
                    args: json!({})
                })
            );
            if let Some(tool) = &spec.mcp_tool {
                assert!(names.contains(&tool.as_str()), "`{tool}` is not offered");
                assert_eq!(
                    route(tool, None, &specs),
                    Ok(Route::Execute {
                        command: spec.id.clone(),
                        args: json!({})
                    })
                );
            }
        }
    }

    #[test]
    fn a_tool_name_is_offered_once() {
        let tools = tools_for(&specs());
        let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        let offered = names.len();
        names.sort_unstable();
        names.dedup();

        assert_eq!(names.len(), offered);
    }

    #[test]
    fn a_query_is_marked_read_only() {
        let tools = tools_for(&specs());
        let version = tools.iter().find(|t| t.name == "get_version").unwrap();

        assert_eq!(
            version.annotations.as_ref().unwrap().read_only_hint,
            Some(true)
        );
        assert!(version.output_schema.is_some());
    }

    #[test]
    fn an_unknown_tool_and_a_missing_command_are_refused() {
        assert!(route("no_such_tool", None, &specs()).is_err());
        assert!(route(EXECUTE_COMMAND, None, &specs()).is_err());
    }
}
