#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Asking each kind of model, against a stand-in that holds requests to
//! what the real ones take.

use brolga_ai::{AiError, Ask, Provider, ask, ollama_models};
use brolga_fake_ai::{FakeAi, KEY, Reply};
use serde_json::json;

fn question() -> Ask {
    Ask {
        system: "Sort the mail.".to_owned(),
        user: "{\"subject\":\"Invoice\"}".to_owned(),
        schema: json!({
            "type": "object",
            "properties": { "folder": { "type": "string", "enum": ["Accounts", ""] } },
            "required": ["folder"],
            "additionalProperties": false
        }),
    }
}

fn anthropic(ai: &FakeAi, key: &str) -> Provider {
    Provider::Anthropic {
        base: ai.base_url(),
        key: key.to_owned(),
        model: "claude-haiku-5-5".to_owned(),
    }
}

fn ollama(ai: &FakeAi, model: &str) -> Provider {
    Provider::Ollama {
        base: ai.base_url(),
        model: model.to_owned(),
    }
}

#[tokio::test]
async fn both_kinds_of_model_answer_in_the_schema() {
    let ai = FakeAi::start().await.unwrap();
    ai.answer_with(|_| Reply::Answer(json!({ "folder": "Accounts" })));
    let http = brolga_ai::client().unwrap();

    for provider in [anthropic(&ai, KEY), ollama(&ai, "llama-test")] {
        let answer = ask(&http, &provider, &question()).await.unwrap();
        assert_eq!(answer.value, json!({ "folder": "Accounts" }));
        assert_eq!(answer.input_tokens, 400);
    }
    let asked = ai.asked();
    assert_eq!(asked[0].provider, "anthropic");
    assert_eq!(asked[0].system, "Sort the mail.");
    assert_eq!(asked[1].provider, "ollama");
    assert_eq!(asked[1].user, "{\"subject\":\"Invoice\"}");
}

#[tokio::test]
async fn an_answer_outside_the_schema_is_not_used() {
    let ai = FakeAi::start().await.unwrap();
    ai.answer_with(|_| Reply::Answer(json!({ "folder": "Deleted Items" })));
    let http = brolga_ai::client().unwrap();

    for provider in [anthropic(&ai, KEY), ollama(&ai, "llama-test")] {
        let error = ask(&http, &provider, &question()).await.unwrap_err();
        assert!(matches!(error, AiError::BadAnswer(_)), "{error}");
    }

    ai.answer_with(|_| Reply::Garbled("Sure! Here you go.".to_owned()));
    let error = ask(&http, &ollama(&ai, "llama-test"), &question())
        .await
        .unwrap_err();
    assert!(matches!(error, AiError::BadAnswer(_)), "{error}");
}

#[tokio::test]
async fn refusals_and_failures_are_told_apart() {
    let ai = FakeAi::start().await.unwrap();
    let http = brolga_ai::client().unwrap();

    ai.answer_with(|_| Reply::Refusal);
    let refused = ask(&http, &anthropic(&ai, KEY), &question())
        .await
        .unwrap_err();
    assert!(matches!(refused, AiError::Refused(_)), "{refused}");
    assert!(!refused.is_transient());

    let wrong_key = ask(&http, &anthropic(&ai, "sk-ant-wrong"), &question())
        .await
        .unwrap_err();
    assert!(matches!(wrong_key, AiError::Rejected(_)), "{wrong_key}");

    ai.answer_with(|_| Reply::Status(529));
    assert!(
        ask(&http, &anthropic(&ai, KEY), &question())
            .await
            .unwrap_err()
            .is_transient()
    );
    ai.answer_with(|_| Reply::Status(429));
    assert!(matches!(
        ask(&http, &anthropic(&ai, KEY), &question())
            .await
            .unwrap_err(),
        AiError::Busy { .. }
    ));

    let missing = ask(&http, &ollama(&ai, "no-such-model"), &question())
        .await
        .unwrap_err();
    assert!(matches!(missing, AiError::Rejected(_)), "{missing}");

    // Nothing listening: worth trying again later.
    let gone = Provider::Ollama {
        base: "http://127.0.0.1:9".to_owned(),
        model: "llama-test".to_owned(),
    };
    assert!(
        ask(&http, &gone, &question())
            .await
            .unwrap_err()
            .is_transient()
    );
}

#[tokio::test]
async fn ollamas_models_are_listed() {
    let ai = FakeAi::start().await.unwrap();
    let http = brolga_ai::client().unwrap();

    assert_eq!(
        ollama_models(&http, &ai.base_url()).await.unwrap(),
        ["llama-test"]
    );
}
