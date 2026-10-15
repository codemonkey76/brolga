//! A stand-in for the two kinds of AI model Brolga asks: Anthropic's
//! Messages API (`POST /v1/messages`) and an Ollama server (`POST /api/chat`,
//! `GET /api/tags`), as their documentation describes them (SOURCES.md).
//!
//! What it answers is up to the test: a function of what was asked. It
//! refuses a request whose form the real one would refuse, so that a
//! mistake in what Brolga sends is caught here and not by a real account.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};

/// The key the stand-in for Anthropic takes. Any other is refused.
pub const KEY: &str = "sk-ant-test-key";

/// One question, as it arrived.
#[derive(Debug, Clone)]
pub struct Asked {
    /// `anthropic` or `ollama`.
    pub provider: &'static str,
    pub model: String,
    pub system: String,
    pub user: String,
    pub schema: Value,
}

/// What to answer with.
#[derive(Debug, Clone)]
pub enum Reply {
    /// This JSON, as the model's answer.
    Answer(Value),
    /// Text that is not JSON.
    Garbled(String),
    /// The model declines.
    Refusal,
    /// The service answers with this status and nothing useful.
    Status(u16),
}

type Brain = Arc<dyn Fn(&Asked) -> Reply + Send + Sync>;

struct Inner {
    brain: Brain,
    asked: Vec<Asked>,
    models: Vec<String>,
}

#[derive(Clone)]
struct Shared(Arc<Mutex<Inner>>);

fn lock(shared: &Shared) -> MutexGuard<'_, Inner> {
    shared
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub struct FakeAi {
    address: SocketAddr,
    shared: Shared,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for FakeAi {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl FakeAi {
    /// A model that does nothing with every message, until told otherwise.
    pub async fn start() -> std::io::Result<Self> {
        let shared = Shared(Arc::new(Mutex::new(Inner {
            brain: Arc::new(|_| Reply::Answer(json!({ "actions": [] }))),
            asked: Vec::new(),
            models: vec!["llama-test".to_owned()],
        })));
        let app = Router::new()
            .route("/v1/messages", post(anthropic))
            .route("/api/chat", post(ollama))
            .route("/api/tags", get(tags))
            .with_state(shared.clone());
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Ok(Self {
            address,
            shared,
            server,
        })
    }

    /// Where it listens, as the base of either API.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// Answer each question with what this gives for it.
    pub fn answer_with(&self, brain: impl Fn(&Asked) -> Reply + Send + Sync + 'static) {
        lock(&self.shared).brain = Arc::new(brain);
    }

    /// Every question asked so far.
    pub fn asked(&self) -> Vec<Asked> {
        lock(&self.shared).asked.clone()
    }
}

fn refused(status: StatusCode, kind: &str, message: &str) -> Response {
    (
        status,
        axum::Json(json!({ "type": "error", "error": { "type": kind, "message": message } })),
    )
        .into_response()
}

/// A schema of the kind structured outputs take ("Structured outputs",
/// SOURCES.md): every object closed, and no limits on length or size.
fn schema_problem(schema: &Value) -> Option<String> {
    match schema {
        Value::Object(map) => {
            for limit in ["minLength", "maxLength", "minimum", "maximum", "multipleOf"] {
                if map.contains_key(limit) {
                    return Some(format!("`{limit}` is not supported"));
                }
            }
            if map.get("type") == Some(&json!("object"))
                && map.get("additionalProperties") != Some(&json!(false))
            {
                return Some("every object must have additionalProperties false".to_owned());
            }
            map.values().find_map(schema_problem)
        }
        Value::Array(items) => items.iter().find_map(schema_problem),
        _ => None,
    }
}

fn think(shared: &Shared, asked: Asked) -> Reply {
    let mut inner = lock(shared);
    inner.asked.push(asked.clone());
    let brain = inner.brain.clone();
    drop(inner);

    brain(&asked)
}

async fn anthropic(
    State(shared): State<Shared>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if headers.get("x-api-key").and_then(|k| k.to_str().ok()) != Some(KEY) {
        return refused(
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            "invalid x-api-key",
        );
    }
    if headers.get("anthropic-version").is_none() {
        return refused(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "anthropic-version header is required",
        );
    }
    let format = &body["output_config"]["format"];
    if body["model"].as_str().is_none()
        || body["max_tokens"].as_u64().is_none()
        || format["type"] != "json_schema"
        || !body["messages"].is_array()
    {
        return refused(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "the request is not one this stand-in takes",
        );
    }
    if let Some(problem) = schema_problem(&format["schema"]) {
        return refused(StatusCode::BAD_REQUEST, "invalid_request_error", &problem);
    }
    let model = body["model"].as_str().unwrap_or_default().to_owned();
    let asked = Asked {
        provider: "anthropic",
        model: model.clone(),
        system: body["system"].as_str().unwrap_or_default().to_owned(),
        user: body["messages"][0]["content"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        schema: format["schema"].clone(),
    };
    let message = |stop: &str, content: Value| {
        axum::Json(json!({
            "id": "msg_test", "type": "message", "role": "assistant", "model": model,
            "content": content, "stop_reason": stop, "stop_details": null,
            "usage": { "input_tokens": 400, "output_tokens": 40 }
        }))
        .into_response()
    };

    match think(&shared, asked) {
        // Thinking comes first, as a block of its own, with nothing in it.
        Reply::Answer(value) => message(
            "end_turn",
            json!([{ "type": "thinking", "thinking": "", "signature": "x" }, { "type": "text", "text": value.to_string() }]),
        ),
        Reply::Garbled(text) => message("end_turn", json!([{ "type": "text", "text": text }])),
        Reply::Refusal => {
            let mut answer = json!({
                "id": "msg_test", "type": "message", "role": "assistant", "model": model,
                "content": [], "stop_reason": "refusal",
                "stop_details": { "type": "refusal", "category": "cyber", "explanation": "declined in a test" },
                "usage": { "input_tokens": 400, "output_tokens": 0 }
            });
            answer["content"] = json!([]);
            axum::Json(answer).into_response()
        }
        Reply::Status(code) => refused(
            StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            "api_error",
            "told to fail by the test",
        ),
    }
}

async fn ollama(State(shared): State<Shared>, axum::Json(body): axum::Json<Value>) -> Response {
    let model = body["model"].as_str().unwrap_or_default().to_owned();
    if !lock(&shared).models.contains(&model) {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(json!({ "error": format!("model '{model}' not found") })),
        )
            .into_response();
    }
    if body["stream"] != false || !body["format"].is_object() {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({ "error": "this stand-in answers only stream false with a schema" })),
        )
            .into_response();
    }
    let said = |role: &str| {
        body["messages"]
            .as_array()
            .and_then(|all| all.iter().find(|m| m["role"] == role))
            .and_then(|m| m["content"].as_str())
            .unwrap_or_default()
            .to_owned()
    };
    let asked = Asked {
        provider: "ollama",
        model: model.clone(),
        system: said("system"),
        user: said("user"),
        schema: body["format"].clone(),
    };
    let message = |content: String| {
        axum::Json(json!({
            "model": model, "created_at": "2026-10-16T00:00:00Z",
            "message": { "role": "assistant", "content": content },
            "done": true, "done_reason": "stop", "prompt_eval_count": 400, "eval_count": 40
        }))
        .into_response()
    };

    match think(&shared, asked) {
        Reply::Answer(value) => message(value.to_string()),
        Reply::Garbled(text) => message(text),
        // A local model has no refusal of its own kind: it says so in words.
        Reply::Refusal => message("I can't help with that.".to_owned()),
        Reply::Status(code) => (
            StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            axum::Json(json!({ "error": "told to fail by the test" })),
        )
            .into_response(),
    }
}

async fn tags(State(shared): State<Shared>) -> Response {
    let models: Vec<Value> = lock(&shared)
        .models
        .iter()
        .map(|name| json!({ "name": name, "model": name, "size": 1, "details": { "family": "test" } }))
        .collect();

    axum::Json(json!({ "models": models })).into_response()
}
