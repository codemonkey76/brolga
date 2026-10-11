//! Opening a link from a message.

use std::sync::Arc;

use brolga_commands::{CommandError, CommandKind, CommandSpec, Registry, RegistryError};
use serde_json::json;

use crate::OpenBrowser;

/// A link from a message is only ever handed to the browser, and only if it
/// is a web address. Nothing opens inside Brolga, and nothing that could
/// start another program on this computer is passed on.
fn web_address(url: &str) -> Option<&str> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let host = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))?;

    // Something to go to, and no control characters or spaces to confuse
    // whatever opens it.
    (host
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '[')
        && !url.chars().any(|c| c.is_control() || c.is_whitespace()))
    .then_some(url)
}

pub(crate) fn register(
    registry: &mut Registry,
    open_browser: OpenBrowser,
) -> Result<(), RegistryError> {
    registry.register(
        CommandSpec {
            id: "app.open_link".to_owned(),
            title: "Open a link".to_owned(),
            description: "Open a web address in the person's own browser. Only http and https addresses are opened; \
                          anything else is refused. Used for links in messages, which never open inside the app."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "url": { "type": "string" } },
                "required": ["url"],
                "additionalProperties": false
            }),
            result_schema: json!({
                "type": "object",
                "properties": { "opened": { "type": "boolean" } },
                "required": ["opened"],
                "additionalProperties": false
            }),
            examples: vec![json!({ "url": "https://example.test/" })],
        },
        Arc::new(move |args| {
            let open_browser = open_browser.clone();
            Box::pin(async move {
                let url = args["url"].as_str().unwrap_or_default();
                let address = web_address(url)
                    .ok_or_else(|| CommandError::Failed("Only web links (http and https) can be opened.".to_owned()))?;
                open_browser(address)
                    .map_err(|e| CommandError::Failed(format!("Your browser could not be opened: {e}")))?;

                Ok(json!({ "opened": true }))
            })
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_addresses_are_opened() {
        for good in [
            "https://example.test/a?b=c#d",
            "http://example.test",
            " HTTPS://Example.test/ ",
            "https://[::1]/",
        ] {
            assert!(web_address(good).is_some(), "{good}");
        }
        for bad in [
            "",
            "example.test",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "mailto:sam@example.test",
            "ms-msdt:/id",
            "https://",
            "https:///etc/passwd",
            "https://example.test/a b",
            "https://example.test/\nfile:///etc/passwd",
            "data:text/html,x",
            "-https://example.test",
            "smb://server/share",
        ] {
            assert!(web_address(bad).is_none(), "{bad}");
        }
    }
}
