//! The model call noal makes, and nothing else about models.
//!
//! This is the only module that names rig. Every function here builds the
//! client per call — it is cheap, and a Worker isolate serves one request at a
//! time — and runs inside `SendWrapper`, because on wasm32 rig's reqwest
//! resolves a `JsFuture`. The rule from `routes::auth` applies: the wrapper
//! goes around the JavaScript-facing call alone.
//!
//! This module knows nothing about what noal asks the model. The preamble and
//! the prompt both come from the caller, so a feature that needs the model
//! only ever names [`text`] or [`structured`].

use axum::http::{HeaderMap, HeaderValue};
use rig_core::client::CompletionClient;
use rig_core::completion::{AssistantContent, CompletionModel};
use rig_core::providers::anthropic;
use send_wrapper::SendWrapper;

use crate::config::Config;
use crate::failure::Failure;

/// The model every call in this module uses.
///
/// One model serves every caller today. A caller that needs a different one
/// takes it as an argument instead of adding a second constant here — that
/// choice belongs to whoever is making the call, not to this module.
pub const PLAN_MODEL: &str = "claude-sonnet-5";

/// Ask the model a question and return its first block of text, unparsed.
///
/// # Errors
///
/// Returns [`Failure::Model`] when the client cannot be built, the call
/// fails, or the answer holds no text.
pub async fn text(config: &Config, preamble: &str, prompt: String) -> Result<String, Failure> {
    let settings = Settings::from(config);
    let preamble = preamble.to_owned();
    SendWrapper::new(async move {
        let client = settings.client()?;
        let model = client.completion_model(PLAN_MODEL);
        let request = model.completion_request(prompt).preamble(preamble).build();
        let response = model.completion(request).await.map_err(Failure::model)?;
        first_text(&response.choice)
    })
    .await
}

/// Ask the model a question and parse its answer as `T`.
///
/// The schema for `T` is sent as the model's output schema, so the answer
/// arrives as a text block holding JSON rather than as a tool call — hence
/// the `serde_json::from_str` rather than reading a structured tool result.
///
/// # Errors
///
/// Returns [`Failure::Model`] when the client cannot be built, the call
/// fails, the answer holds no text, or the text does not parse as `T`.
pub async fn structured<T>(config: &Config, preamble: &str, prompt: String) -> Result<T, Failure>
where
    T: serde::de::DeserializeOwned + schemars::JsonSchema,
{
    let settings = Settings::from(config);
    let preamble = preamble.to_owned();
    SendWrapper::new(async move {
        let client = settings.client()?;
        let model = client.completion_model(PLAN_MODEL);
        let request = model
            .completion_request(prompt)
            .preamble(preamble)
            .output_schema(schemars::schema_for!(T))
            .build();
        let response = model.completion(request).await.map_err(Failure::model)?;
        let text = first_text(&response.choice)?;
        serde_json::from_str(&text).map_err(Failure::model)
    })
    .await
}

/// What a call needs, copied out of [`Config`] so the future owns it and does
/// not borrow across the `SendWrapper` boundary.
struct Settings {
    /// The Anthropic API key.
    api_key: String,
    /// The Cloudflare AI Gateway token, when calls go through a gateway.
    gateway_token: Option<String>,
    /// Where the Anthropic API is reached.
    base_url: String,
}

impl From<&Config> for Settings {
    fn from(config: &Config) -> Self {
        Self {
            api_key: config.anthropic_api_key.clone(),
            gateway_token: config.ai_gateway_token.clone(),
            base_url: config.llm_base_url.clone(),
        }
    }
}

impl Settings {
    /// Build the Anthropic client, through the gateway when a token is set.
    fn client(&self) -> Result<anthropic::Client, Failure> {
        let mut headers = HeaderMap::new();
        if let Some(token) = &self.gateway_token {
            let value = HeaderValue::from_str(token).map_err(Failure::model)?;
            headers.insert("cf-aig-authorization", value);
        }
        anthropic::Client::builder()
            .api_key(self.api_key.as_str())
            .base_url(self.base_url.as_str())
            .http_headers(headers)
            .build()
            .map_err(Failure::model)
    }
}

/// The first text block of an answer.
fn first_text(choice: &[AssistantContent]) -> Result<String, Failure> {
    choice
        .iter()
        .find_map(|content| match content {
            AssistantContent::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .ok_or_else(|| Failure::Model("the answer had no text".to_owned()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::first_text;
    use rig_core::completion::message::{Text, ToolCall, ToolCallId, ToolFunction};
    use rig_core::completion::AssistantContent;

    fn tool_call() -> AssistantContent {
        let id = ToolCallId::new("call-1").expect("non-empty id");
        let function = ToolFunction {
            name: "irrelevant".to_owned(),
            arguments: serde_json::json!({}),
        };
        AssistantContent::ToolCall(ToolCall::new(id, function))
    }

    #[test]
    fn returns_the_first_text_block() {
        let choice = vec![AssistantContent::Text(Text {
            text: "ok".to_owned(),
            additional_params: None,
        })];

        assert_eq!(first_text(&choice).unwrap(), "ok");
    }

    #[test]
    fn skips_a_leading_tool_call_to_find_the_text_after_it() {
        let choice = vec![
            tool_call(),
            AssistantContent::Text(Text {
                text: "ok".to_owned(),
                additional_params: None,
            }),
        ];

        assert_eq!(first_text(&choice).unwrap(), "ok");
    }

    #[test]
    fn errors_when_there_is_no_text_block() {
        let choice = vec![tool_call()];

        assert!(first_text(&choice).is_err());
    }

    #[test]
    fn errors_on_an_empty_answer() {
        assert!(first_text(&[]).is_err());
    }
}
