use anyhow::Result;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc::UnboundedSender;

use crate::app::AppEvent;
use crate::config::{LmConfig, Params};

#[derive(Serialize)]
struct ChatMessage<'a> {
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

/// Stream a chat completion from LM Studio's OpenAI-compatible endpoint,
/// forwarding token deltas to the UI as they arrive.
pub async fn stream_chat(cfg: LmConfig, params: Params, prompt: String, tx: UnboundedSender<AppEvent>) {
    if let Err(err) = run(cfg, params, prompt, &tx).await {
        let _ = tx.send(AppEvent::LmError(err.to_string()));
    } else {
        let _ = tx.send(AppEvent::LmDone);
    }
}

async fn run(
    cfg: LmConfig,
    params: Params,
    prompt: String,
    tx: &UnboundedSender<AppEvent>,
) -> Result<()> {
    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));

    let body = json!({
        "model": cfg.model,
        "stream": true,
        "temperature": params.temperature,
        "max_tokens": params.max_tokens.round() as i64,
        "top_p": params.top_p,
        "messages": [ChatMessage { role: "user", content: &prompt }],
    });

    let resp = client.post(&url).json(&body).send().await?;
    let resp = resp.error_for_status()?;

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        buf.push_str(&String::from_utf8_lossy(&bytes));

        // SSE frames are separated by blank lines; process complete lines.
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
                            let _ = tx.send(AppEvent::LmChunk(text));
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
