use brolga_commands::CommandError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The longest line either side will read. A line that runs past it is not a
/// request, and the connection is closed rather than read into memory.
pub const MAX_LINE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Request {
    /// Sent back unchanged, so a caller can match answers to questions.
    #[serde(default)]
    pub id: Value,
    #[serde(flatten)]
    pub call: Call,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Call {
    ListCommands,
    Execute {
        command: String,
        #[serde(default = "empty_object")]
        args: Value,
    },
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub id: Value,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Result(Value),
    Error(ErrorBody),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

impl From<&CommandError> for ErrorBody {
    fn from(error: &CommandError) -> Self {
        Self {
            code: error.code().to_owned(),
            message: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_request_reads_as_the_documented_json() {
        let request: Request = serde_json::from_value(
            json!({ "id": 7, "method": "execute", "params": { "command": "app.version" } }),
        )
        .unwrap();

        assert_eq!(
            request,
            Request {
                id: json!(7),
                call: Call::Execute {
                    command: "app.version".to_owned(),
                    args: json!({})
                }
            }
        );
        assert_eq!(
            serde_json::from_value::<Request>(json!({ "method": "list_commands" })).unwrap(),
            Request {
                id: Value::Null,
                call: Call::ListCommands
            }
        );
    }

    #[test]
    fn a_response_writes_as_the_documented_json() {
        let ok = Response {
            id: json!("a"),
            outcome: Outcome::Result(json!({ "x": 1 })),
        };
        let failed = Response {
            id: json!(2),
            outcome: Outcome::Error(ErrorBody {
                code: "failed".to_owned(),
                message: "no".to_owned(),
            }),
        };

        assert_eq!(
            serde_json::to_value(ok).unwrap(),
            json!({ "id": "a", "result": { "x": 1 } })
        );
        assert_eq!(
            serde_json::to_value(failed).unwrap(),
            json!({ "id": 2, "error": { "code": "failed", "message": "no" } })
        );
    }
}
