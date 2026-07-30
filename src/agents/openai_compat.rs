use std::time::Instant;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc::UnboundedSender;

use crate::app::AppEvent;
use crate::config::{HttpConfig, Params};
use crate::session::{ChatMessage, Role, RunNode, RunStatus, now_rfc3339};

#[derive(Serialize)]
struct ApiMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: Delta,
}

#[derive(Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
}

/// Stream a chat completion from an OpenAI-compatible endpoint.
pub async fn stream_chat(
    session_id: String,
    cfg: HttpConfig,
    params: Params,
    messages: Vec<ChatMessage>,
    tx: UnboundedSender<AppEvent>,
) {
    let started = Instant::now();
    let mut run = RunNode::new(cfg.model.clone(), messages.clone());

    match run_inner(&cfg, &params, &messages, &tx, &session_id, &mut run.response).await {
        Ok(()) => {
            run.status = RunStatus::Done;
            run.duration_ms = started.elapsed().as_millis() as u64;
            run.started_at = now_rfc3339();
            let _ = tx.send(AppEvent::RunFinished(session_id, run));
        }
        Err(err) => {
            run.status = RunStatus::Error;
            run.duration_ms = started.elapsed().as_millis() as u64;
            run.error = Some(err.to_string());
            let _ = tx.send(AppEvent::RunFinished(session_id.clone(), run));
            let _ = tx.send(AppEvent::HttpError(session_id, err.to_string()));
        }
    }
}

async fn run_inner(
    cfg: &HttpConfig,
    params: &Params,
    messages: &[ChatMessage],
    tx: &UnboundedSender<AppEvent>,
    session_id: &str,
    response: &mut String,
) -> Result<()> {
    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));

    let mut api_messages: Vec<ApiMessage> = Vec::new();
    if !cfg.system_prompt.is_empty() {
        api_messages.push(ApiMessage {
            role: "system",
            content: &cfg.system_prompt,
        });
    }
    for m in messages {
        let role = match m.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        api_messages.push(ApiMessage {
            role,
            content: &m.content,
        });
    }

    let body = json!({
        "model": cfg.model,
        "stream": true,
        "temperature": params.temperature,
        "max_tokens": params.max_tokens.round() as i64,
        "top_p": params.top_p,
        "messages": api_messages,
    });

    let mut req = client.post(&url).json(&body);
    if let Some(env_key) = &cfg.api_key_env {
        if let Ok(key) = std::env::var(env_key) {
            let mut headers = HeaderMap::new();
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {key}"))
                    .context("invalid api key header")?,
            );
            req = req.headers(headers);
        }
    }

    let resp = req.send().await?;
    let resp = resp.error_for_status()?;

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        buf.push_str(&String::from_utf8_lossy(&bytes));

        while let Some(idx) = buf.find('\n') {
            let line = buf[..idx].trim().to_string();
            buf.drain(..=idx);

            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data.is_empty() {
                continue;
            }
            if data == "[DONE]" {
                return Ok(());
            }
            if let Ok(parsed) = serde_json::from_str::<StreamChunk>(data) {
                for choice in parsed.choices {
                    if let Some(text) = choice.delta.content {
                        if !text.is_empty() {
                            response.push_str(&text);
                            let _ = tx.send(AppEvent::HttpChunk(session_id.to_string(), text));
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Parse SSE lines from a buffer (used in tests).
pub fn parse_sse_line(data: &str, out: &mut String) -> bool {
    if data == "[DONE]" {
        return true;
    }
    if let Ok(parsed) = serde_json::from_str::<StreamChunk>(data) {
        for choice in parsed.choices {
            if let Some(text) = choice.delta.content {
                if !text.is_empty() {
                    out.push_str(&text);
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::parse_sse_line;

    #[test]
    fn parses_sse_delta() {
        let data = r#"{"choices":[{"delta":{"content":"Hello"}}]}"#;
        let mut out = String::new();
        assert!(!parse_sse_line(data, &mut out));
        assert_eq!(out, "Hello");
    }

    #[test]
    fn parses_sse_done() {
        let mut out = String::new();
        assert!(parse_sse_line("[DONE]", &mut out));
    }
}
