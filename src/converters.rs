//! Format converters for different API formats.

use crate::config::ApiFormat;
use serde_json::{json, Value};

/// Convert Anthropic request to target format.
/// Returns (converted_body, endpoint_path).
pub fn to_upstream(body: &Value, fmt: &ApiFormat) -> (Value, &'static str) {
    match fmt {
        ApiFormat::Anthropic => (body.clone(), "/messages"),

        ApiFormat::Openai => {
            let mut messages = vec![];

            // Move system prompt to messages
            if let Some(system) = body.get("system") {
                messages.push(json!({
                    "role": "system",
                    "content": system
                }));
            }

            // Convert messages
            if let Some(msgs) = body.get("messages").and_then(|m| m.as_array()) {
                for msg in msgs {
                    let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                    let content = if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
                        content_arr
                            .iter()
                            .filter_map(|c| {
                                if c.get("type")?.as_str()? == "text" {
                                    c.get("text")?.as_str()
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    } else {
                        msg.get("content")
                            .and_then(|c| c.as_str())
                            .unwrap_or("")
                            .to_string()
                    };

                    messages.push(json!({
                        "role": role,
                        "content": content
                    }));
                }
            }

            let mut converted = json!({
                "model": body.get("model").and_then(|m| m.as_str()).unwrap_or("gpt-4"),
                "messages": messages,
                "stream": body.get("stream").and_then(|s| s.as_bool()).unwrap_or(false)
            });

            if let Some(max_tokens) = body.get("max_tokens") {
                converted["max_completion_tokens"] = max_tokens.clone();
            }

            (converted, "/chat/completions")
        }

        ApiFormat::OpenaiResponse => {
            let mut converted = json!({
                "model": body.get("model").and_then(|m| m.as_str()).unwrap_or("gpt-4"),
                "input": body.get("messages").cloned().unwrap_or(json!([])),
                "stream": body.get("stream").and_then(|s| s.as_bool()).unwrap_or(false)
            });

            if let Some(max_tokens) = body.get("max_tokens") {
                converted["max_tokens"] = max_tokens.clone();
            }

            (converted, "/responses")
        }

        ApiFormat::Gemini => {
            let mut contents = vec![];

            if let Some(msgs) = body.get("messages").and_then(|m| m.as_array()) {
                for msg in msgs {
                    let role = if msg.get("role").and_then(|r| r.as_str()) == Some("assistant") {
                        "model"
                    } else {
                        "user"
                    };

                    let parts = if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
                        content_arr
                            .iter()
                            .filter_map(|c| {
                                if c.get("type")?.as_str()? == "text" {
                                    Some(json!({ "text": c.get("text")? }))
                                } else {
                                    None
                                }
                            })
                            .collect()
                    } else {
                        vec![json!({ "text": msg.get("content").and_then(|c| c.as_str()).unwrap_or("") })]
                    };

                    contents.push(json!({
                        "role": role,
                        "parts": parts
                    }));
                }
            }

            let mut converted = json!({
                "contents": contents,
                "generationConfig": {}
            });

            if let Some(system) = body.get("system") {
                converted["systemInstruction"] = json!({
                    "parts": [{ "text": system }]
                });
            }

            if let Some(max_tokens) = body.get("max_tokens") {
                converted["generationConfig"]["maxOutputTokens"] = max_tokens.clone();
            }

            let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("gemini-pro");
            let _endpoint = format!("/models/{}:generateContent", model);
            // Note: This is a static str for consistency, but we need to handle this differently
            // For now, just return a default endpoint
            (converted, "/models/gemini-pro:generateContent")
        }
    }
}

/// Convert upstream response back to Anthropic format.
pub fn from_upstream(body: &Value, fmt: &ApiFormat) -> Value {
    match fmt {
        ApiFormat::Anthropic => body.clone(),

        ApiFormat::Openai => {
            let choice = body.get("choices").and_then(|c| c.as_array()).and_then(|a| a.first());
            let message = choice.and_then(|c| c.get("message"));
            let content = message
                .and_then(|m| m.get("content").and_then(|c| c.as_str()))
                .unwrap_or("");

            json!({
                "id": body.get("id").and_then(|i| i.as_str()).unwrap_or("msg-unknown"),
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "text", "text": content }],
                "model": body.get("model").and_then(|m| m.as_str()).unwrap_or("unknown"),
                "stop_reason": if choice.and_then(|c| c.get("finish_reason")).and_then(|f| f.as_str()) == Some("stop") {
                    "end_turn"
                } else {
                    "max_tokens"
                },
                "usage": {
                    "input_tokens": body.get("usage").and_then(|u| u.get("prompt_tokens")).and_then(|p| p.as_u64()).unwrap_or(0),
                    "output_tokens": body.get("usage").and_then(|u| u.get("completion_tokens")).and_then(|c| c.as_u64()).unwrap_or(0)
                }
            })
        }

        ApiFormat::OpenaiResponse => {
            let output = body
                .get("output")
                .and_then(|o| o.as_array())
                .and_then(|a| a.last())
                .and_then(|o| o.get("content").and_then(|c| c.as_str()))
                .unwrap_or("");

            json!({
                "id": body.get("id").and_then(|i| i.as_str()).unwrap_or("msg-unknown"),
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "text", "text": output }],
                "model": body.get("model").and_then(|m| m.as_str()).unwrap_or("unknown"),
                "stop_reason": "end_turn",
                "usage": body.get("usage").cloned().unwrap_or(json!({}))
            })
        }

        ApiFormat::Gemini => {
            let candidate = body.get("candidates").and_then(|c| c.as_array()).and_then(|a| a.first());
            let parts = candidate
                .and_then(|c| c.get("content"))
                .and_then(|c| c.get("parts"))
                .and_then(|p| p.as_array());

            let text = parts
                .map(|p| {
                    p.iter()
                        .filter_map(|part| part.get("text").and_then(|t| t.as_str()))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();

            json!({
                "id": "msg-gemini",
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "text", "text": text }],
                "model": body.get("modelVersion").and_then(|m| m.as_str()).unwrap_or("unknown"),
                "stop_reason": "end_turn",
                "usage": {
                    "input_tokens": body.get("usageMetadata").and_then(|u| u.get("promptTokenCount")).and_then(|p| p.as_u64()).unwrap_or(0),
                    "output_tokens": body.get("usageMetadata").and_then(|u| u.get("candidatesTokenCount")).and_then(|c| c.as_u64()).unwrap_or(0)
                }
            })
        }
    }
}

/// Convert a single SSE chunk from upstream format to Anthropic format.
pub fn convert_stream_chunk(chunk: &str, fmt: &ApiFormat) -> Option<String> {
    if *fmt == ApiFormat::Anthropic {
        return Some(chunk.to_string());
    }

    let chunk = chunk.trim();
    if !chunk.starts_with("data: ") {
        return Some(chunk.to_string());
    }

    let json_str = &chunk[6..];
    if json_str == "[DONE]" {
        return Some("data: {\"type\":\"message_stop\"}\n\n".to_string());
    }

    let data: Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return Some(chunk.to_string()),
    };

    let content = match fmt {
        ApiFormat::Openai => {
            data.get("choices")
                .and_then(|c| c.as_array())
                .and_then(|a| a.first())
                .and_then(|c| c.get("delta"))
                .and_then(|d| d.get("content"))
                .and_then(|c| c.as_str())
                .map(|s| s.to_string())
        }
        ApiFormat::Gemini => {
            let parts = data
                .get("candidates")
                .and_then(|c| c.as_array())
                .and_then(|a| a.first())
                .and_then(|c| c.get("content"))
                .and_then(|c| c.get("parts"))
                .and_then(|p| p.as_array());

            parts.map(|p| {
                p.iter()
                    .filter_map(|part| part.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
        }
        _ => return Some(chunk.to_string()),
    };

    if let Some(text) = content {
        if !text.is_empty() {
            let anthropic_chunk = json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "text_delta", "text": text }
            });
            return Some(format!("data: {}\n\n", anthropic_chunk));
        }
    }

    None
}
