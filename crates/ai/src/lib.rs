//! Asks an AI model one question and gets back one answer, held to a JSON
//! schema: Anthropic's Messages API, or a local Ollama server
//! (docs/spec-mail-instructions.md, "Models").
//!
//! There is no conversation and no tool use. The answer is checked against
//! the schema here, whichever provider gave it, so that a model that does
//! not keep to it is caught before anything is done with what it said.

use std::time::Duration;

use serde_json::{Value, json};
use url::Url;

/// Where Anthropic's API is. Tests name a stand-in; nothing else may.
pub const ANTHROPIC: &str = "https://api.anthropic.com";

/// Where Ollama listens unless told otherwise.
pub const OLLAMA: &str = "http://localhost:11434";

/// The model a mailbox uses at Anthropic until another is chosen.
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-haiku-5-5";

/// The version of the Messages API the requests are written for.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Room for the answer, and for any thinking the model does first, which
/// counts towards it ("Claude Haiku 5.5 migration guide", SOURCES.md).
const MOST_TOKENS: u32 = 8_192;

/// Which model, and where.
#[derive(Clone)]
pub enum Provider {
    Anthropic {
        /// [`ANTHROPIC`], except in tests.
        base: String,
        key: String,
        model: String,
    },
    Ollama {
        base: String,
        model: String,
    },
}

impl Provider {
    pub fn model(&self) -> &str {
        match self {
            Self::Anthropic { model, .. } | Self::Ollama { model, .. } => model,
        }
    }

    /// `anthropic` or `ollama`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Anthropic { .. } => "anthropic",
            Self::Ollama { .. } => "ollama",
        }
    }
}

impl std::fmt::Debug for Provider {
    // The key is never written out, not even to a log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.name(), self.model())
    }
}

/// The question: what the model is to do, the matter it is to do it to,
/// and the form its answer must take.
pub struct Ask {
    pub system: String,
    pub user: String,
    /// A JSON Schema of the subset both providers take: every object closed
    /// with `additionalProperties: false`, no length or number limits.
    pub schema: Value,
}

/// The answer, and what it cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub value: Value,
    /// The model that gave it, as the provider names it.
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    /// Not reached, or busy, or broken for now: worth asking again later.
    #[error("{0}")]
    Unreached(String),
    /// Too many requests. Ask again after so many seconds, where it says.
    #[error("the model's service says to slow down{}", .retry_after.map(|s| format!(" for {s} seconds")).unwrap_or_default())]
    Busy { retry_after: Option<u64> },
    /// The model declined to answer.
    #[error("the model declined to answer: {0}")]
    Refused(String),
    /// An answer that is not one: not JSON, not in the schema, or cut short.
    #[error("the model's answer could not be used: {0}")]
    BadAnswer(String),
    /// Refused for a reason asking again will not change: a wrong key, a
    /// model that is not there, no credit left.
    #[error("{0}")]
    Rejected(String),
}

impl AiError {
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Unreached(_) | Self::Busy { .. })
    }
}

/// A client for asking, with time enough for a local model to load.
pub fn client() -> Result<reqwest::Client, AiError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| AiError::Unreached(e.to_string()))
}

/// Ask, and give back the answer once it is known to fit the schema.
pub async fn ask(
    http: &reqwest::Client,
    provider: &Provider,
    ask: &Ask,
) -> Result<Answer, AiError> {
    let answer = match provider {
        Provider::Anthropic { base, key, model } => anthropic(http, base, key, model, ask).await?,
        Provider::Ollama { base, model } => ollama(http, base, model, ask).await?,
    };
    fits(&ask.schema, &answer.value)?;

    Ok(answer)
}

fn fits(schema: &Value, value: &Value) -> Result<(), AiError> {
    let validator = jsonschema::validator_for(schema)
        .map_err(|e| AiError::BadAnswer(format!("the schema asked for is not one: {e}")))?;
    if let Some(error) = validator.iter_errors(value).next() {
        return Err(AiError::BadAnswer(format!(
            "it does not fit the form asked for: {error}"
        )));
    }

    Ok(())
}

/// "Structured outputs" (SOURCES.md): the schema goes in
/// `output_config.format`, and the JSON comes back as the text of the
/// answer. Thinking and effort are left to the model's own defaults, which
/// every current model takes.
async fn anthropic(
    http: &reqwest::Client,
    base: &str,
    key: &str,
    model: &str,
    ask: &Ask,
) -> Result<Answer, AiError> {
    let body = json!({
        "model": model,
        "max_tokens": MOST_TOKENS,
        "system": ask.system,
        "messages": [{ "role": "user", "content": ask.user }],
        "output_config": { "format": { "type": "json_schema", "schema": ask.schema } }
    });
    let response = http
        .post(format!("{}/v1/messages", base.trim_end_matches('/')))
        .header("x-api-key", key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .json(&body)
        .send()
        .await
        .map_err(|e| AiError::Unreached(format!("Anthropic could not be reached: {e}")))?;
    let status = response.status();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok());
    let text = response
        .text()
        .await
        .map_err(|e| AiError::Unreached(format!("Anthropic's answer was cut off: {e}")))?;
    let answer: Value = serde_json::from_str(&text).unwrap_or(Value::Null);

    if !status.is_success() {
        // "Errors" (SOURCES.md): `{"type":"error","error":{"type","message"}}`.
        let said = answer["error"]["message"]
            .as_str()
            .unwrap_or("no reason was given")
            .to_owned();
        return Err(match status.as_u16() {
            429 if answer["error"]["details"]["error_code"] == "enforced_spend_limit_reached" => {
                AiError::Rejected(format!(
                    "The Anthropic account has reached its spending limit: {said}"
                ))
            }
            429 => AiError::Busy { retry_after },
            401 => AiError::Rejected(format!("Anthropic did not accept the API key: {said}")),
            402 => AiError::Rejected(format!("The Anthropic account cannot be billed: {said}")),
            403 => AiError::Rejected(format!("The API key may not do this: {said}")),
            404 => AiError::Rejected(format!("Anthropic has no model `{model}`: {said}")),
            408 | 500..=599 => AiError::Unreached(format!(
                "Anthropic is not answering properly just now ({status}): {said}"
            )),
            _ => AiError::Rejected(format!("Anthropic refused the request ({status}): {said}")),
        });
    }

    match answer["stop_reason"].as_str() {
        Some("refusal") => {
            let why = answer["stop_details"]["explanation"]
                .as_str()
                .or_else(|| answer["stop_details"]["category"].as_str())
                .unwrap_or("no reason was given");
            return Err(AiError::Refused(why.to_owned()));
        }
        Some("max_tokens") => {
            return Err(AiError::BadAnswer(
                "it ran out of room before it was finished".to_owned(),
            ));
        }
        _ => {}
    }
    // The JSON is in the text; thinking, if any, comes in blocks of its own.
    let written: String = answer["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect()
        })
        .unwrap_or_default();

    Ok(Answer {
        value: serde_json::from_str(&written)
            .map_err(|e| AiError::BadAnswer(format!("it is not JSON: {e}")))?,
        model: answer["model"].as_str().unwrap_or(model).to_owned(),
        input_tokens: answer["usage"]["input_tokens"].as_u64().unwrap_or_default(),
        output_tokens: answer["usage"]["output_tokens"]
            .as_u64()
            .unwrap_or_default(),
    })
}

/// "Chat" and "Structured outputs" in Ollama's documentation (SOURCES.md):
/// the schema goes in `format`, and the JSON comes back as the message's
/// text. At temperature 0, as those pages advise for an answer of a form.
async fn ollama(
    http: &reqwest::Client,
    base: &str,
    model: &str,
    ask: &Ask,
) -> Result<Answer, AiError> {
    let base = checked_ollama(base).map_err(AiError::Rejected)?;
    let body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": ask.system },
            { "role": "user", "content": ask.user }
        ],
        "format": ask.schema,
        "stream": false,
        "options": { "temperature": 0 }
    });
    let response = http
        .post(format!("{}/api/chat", base.as_str().trim_end_matches('/')))
        .json(&body)
        .send()
        .await
        .map_err(|e| AiError::Unreached(format!("Ollama could not be reached at {base}: {e}")))?;
    let status = response.status();
    let answer: Value = response
        .json()
        .await
        .map_err(|e| AiError::Unreached(format!("Ollama's answer could not be read: {e}")))?;
    if !status.is_success() {
        let said = answer["error"].as_str().unwrap_or("no reason was given");
        return Err(match status.as_u16() {
            404 => AiError::Rejected(format!("Ollama has no model `{model}`: {said}")),
            500..=599 => AiError::Unreached(format!(
                "Ollama is not answering properly ({status}): {said}"
            )),
            _ => AiError::Rejected(format!("Ollama refused the request ({status}): {said}")),
        });
    }
    if answer["done_reason"] == "length" {
        return Err(AiError::BadAnswer(
            "it ran out of room before it was finished".to_owned(),
        ));
    }
    let written = answer["message"]["content"].as_str().unwrap_or_default();

    Ok(Answer {
        value: serde_json::from_str(written)
            .map_err(|e| AiError::BadAnswer(format!("it is not JSON: {e}")))?,
        model: answer["model"].as_str().unwrap_or(model).to_owned(),
        input_tokens: answer["prompt_eval_count"].as_u64().unwrap_or_default(),
        output_tokens: answer["eval_count"].as_u64().unwrap_or_default(),
    })
}

/// The models an Ollama server has, by name ("List models", SOURCES.md).
pub async fn ollama_models(http: &reqwest::Client, base: &str) -> Result<Vec<String>, AiError> {
    let base = checked_ollama(base).map_err(AiError::Rejected)?;
    let answer: Value = http
        .get(format!("{}/api/tags", base.as_str().trim_end_matches('/')))
        .send()
        .await
        .map_err(|e| AiError::Unreached(format!("Ollama could not be reached at {base}: {e}")))?
        .error_for_status()
        .map_err(|e| AiError::Unreached(e.to_string()))?
        .json()
        .await
        .map_err(|e| AiError::Unreached(format!("Ollama's answer could not be read: {e}")))?;

    Ok(answer["models"]
        .as_array()
        .map(|all| {
            all.iter()
                .filter_map(|m| m["name"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default())
}

/// An Ollama server's address, if it is one mail may be sent to: on this
/// computer, or anywhere over `https` (docs/spec-mail-instructions.md,
/// "Ollama").
pub fn checked_ollama(address: &str) -> Result<Url, String> {
    let url = Url::parse(address.trim())
        .map_err(|_| format!("`{address}` is not a web address, such as {OLLAMA}."))?;
    let local = match url.host() {
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => return Err(format!("`{address}` names no computer.")),
    };
    match url.scheme() {
        "https" => {}
        "http" if local => {}
        "http" => {
            return Err(
                "An Ollama server on another computer must be reached over https, so that mail is not sent across the network as it is."
                    .to_owned(),
            );
        }
        _ => return Err(format!("`{address}` is not an http or https address.")),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(
            "Give the Ollama server's address without a name or password in it.".to_owned(),
        );
    }

    Ok(url)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn mail_goes_to_ollama_only_on_this_computer_or_over_https() {
        assert!(checked_ollama(OLLAMA).is_ok());
        assert!(checked_ollama("http://127.0.0.1:11434").is_ok());
        assert!(checked_ollama("http://[::1]:11434").is_ok());
        assert!(checked_ollama("https://ollama.example.com").is_ok());
        assert!(checked_ollama("http://192.168.1.20:11434").is_err());
        assert!(checked_ollama("http://ollama.example.com").is_err());
        assert!(checked_ollama("ftp://localhost").is_err());
        assert!(checked_ollama("https://me:secret@ollama.example.com").is_err());
        assert!(checked_ollama("not an address").is_err());
    }

    #[test]
    fn the_key_is_never_written_out() {
        let provider = Provider::Anthropic {
            base: ANTHROPIC.to_owned(),
            key: "sk-ant-secret".to_owned(),
            model: DEFAULT_ANTHROPIC_MODEL.to_owned(),
        };
        assert!(!format!("{provider:?}").contains("secret"));
    }

    #[test]
    fn an_answer_outside_the_schema_is_caught() {
        let schema = json!({
            "type": "object",
            "properties": { "move": { "type": "string", "enum": ["Accounts"] } },
            "required": ["move"],
            "additionalProperties": false
        });
        assert!(fits(&schema, &json!({ "move": "Accounts" })).is_ok());
        assert!(fits(&schema, &json!({ "move": "Deleted Items" })).is_err());
        assert!(fits(&schema, &json!({ "move": "Accounts", "send": true })).is_err());
    }
}
