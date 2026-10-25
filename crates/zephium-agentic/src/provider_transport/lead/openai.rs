//! OpenAI Responses: streamed tool calls, reasoning items replayed encrypted
//! (`store: false`), and the native `web_search` tool.

use serde_json::{json, Map, Value};
use zephium_core::work::model::*;

use super::sse::SseEvent;
use super::*;

const WIRE: &str = "openai";

pub(super) fn body(
    request: &WorkModelRequest,
    target: &LeadTarget,
) -> Result<Value, WorkModelError> {
    let mut input = Vec::new();
    for message in &request.messages {
        match message {
            WorkModelMessage::User(parts) => {
                let mut content = Vec::new();
                for part in parts {
                    match part {
                        WorkModelPart::Text(text) => {
                            content.push(json!({"type": "input_text", "text": text}))
                        }
                        WorkModelPart::Image { media_type, bytes } => {
                            media_type_ok(media_type)?;
                            content.push(json!({
                                "type": "input_image",
                                "image_url": format!("data:{media_type};base64,{}", image_base64(bytes)?),
                            }));
                        }
                        WorkModelPart::ToolCall(_) | WorkModelPart::Replay(_) => {}
                    }
                }
                if !content.is_empty() {
                    input.push(json!({"type": "message", "role": "user", "content": content}));
                }
            }
            WorkModelMessage::Assistant(parts) => {
                for part in parts {
                    match part {
                        WorkModelPart::Text(text) if !text.is_empty() => input.push(json!({
                            "type": "message",
                            "role": "assistant",
                            "content": [{"type": "output_text", "text": text, "annotations": []}],
                        })),
                        WorkModelPart::ToolCall(call) => input.push(json!({
                            "type": "function_call",
                            "call_id": call.id,
                            "name": call.name,
                            "arguments": serde_json::to_string(&call.arguments)
                                .map_err(|_| WorkModelError::BadRequest)?,
                        })),
                        part => {
                            if let Some(item) = replayed(part, WIRE) {
                                input.push(item.clone());
                            }
                        }
                    }
                }
            }
            WorkModelMessage::ToolResults(results) => {
                for result in results {
                    let output = if result.is_error {
                        format!("Error: {}", result.content)
                    } else {
                        result.content.clone()
                    };
                    input.push(json!({
                        "type": "function_call_output",
                        "call_id": result.call,
                        "output": output,
                    }));
                }
            }
        }
    }
    // Only OpenAI's own endpoints are known to hold a call to a strict schema.
    let strict_provider = matches!(
        request.model.provider,
        WorkModelProvider::OpenAi | WorkModelProvider::Cloud
    );
    let mut tools: Vec<Value> = request
        .tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.schema,
                "strict": tool.strict && strict_provider,
            })
        })
        .collect();
    let native_search = target.native_search(request);
    if native_search {
        tools.push(json!({"type": "web_search"}));
    }
    let mut body = Map::new();
    body.insert("model".into(), json!(request.model.model));
    body.insert("instructions".into(), json!(system_text(request)));
    body.insert("input".into(), Value::Array(input));
    body.insert("stream".into(), json!(true));
    body.insert("store".into(), json!(false));
    // OpenAI, directly or through Zephium Cloud; xAI speaks this wire without the key.
    if matches!(
        request.model.provider,
        WorkModelProvider::OpenAi | WorkModelProvider::Cloud
    ) {
        if let Some(key) = &request.cache_key {
            body.insert("prompt_cache_key".into(), json!(key));
        }
    }
    body.insert("max_output_tokens".into(), json!(request.max_output_tokens));
    if let Some(calls) = target.search_calls.filter(|_| native_search) {
        body.insert("max_tool_calls".into(), json!(calls));
    }
    let mut include = vec![json!("reasoning.encrypted_content")];
    // xAI's Responses API documents no search include.
    if native_search && target.upstream() != WorkModelProvider::XAi {
        include.push(json!("web_search_call.action.sources"));
    }
    body.insert("include".into(), Value::Array(include));
    if let Some(reasoning) = request.reasoning {
        let effort = match reasoning {
            WorkModelReasoning::Low => "low",
            WorkModelReasoning::Medium => "medium",
            WorkModelReasoning::High => "high",
            WorkModelReasoning::XHigh => "xhigh",
            WorkModelReasoning::Max => "max",
        };
        body.insert("reasoning".into(), json!({"effort": effort}));
    }
    if !tools.is_empty() {
        let choice = match tool_choice(request)? {
            WorkModelToolChoice::Auto => json!("auto"),
            WorkModelToolChoice::Required => json!("required"),
            WorkModelToolChoice::Named(name) => json!({"type": "function", "name": name}),
        };
        body.insert("tools".into(), Value::Array(tools));
        body.insert("tool_choice".into(), choice);
        body.insert("parallel_tool_calls".into(), json!(request.parallel_tools));
    }
    Ok(Value::Object(body))
}

#[derive(Default)]
pub(super) struct Decoder {
    assistant: Vec<WorkModelPart>,
    searches: Searches,
    tool_calls: usize,
    refused: bool,
    usage: WorkModelUsage,
    stop: Option<WorkModelStop>,
    text_bytes: usize,
}

impl Decoder {
    pub(super) fn event(
        &mut self,
        frame: &SseEvent,
        out: &mut Vec<WorkModelEvent>,
    ) -> Result<Flow, WorkModelError> {
        if frame.data == "[DONE]" {
            return Ok(Flow::Done);
        }
        let data = json(&frame.data)?;
        let kind = data
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(frame.event.as_str());
        match kind {
            "response.output_text.delta" => {
                if let Some(delta) = data.get("delta").and_then(Value::as_str) {
                    self.text_bytes += delta.len();
                    if self.text_bytes > MAX_TEXT_BYTES {
                        return Err(WorkModelError::Protocol);
                    }
                    if !delta.is_empty() {
                        out.push(WorkModelEvent::Text(delta.to_owned()));
                    }
                }
            }
            "response.output_item.done" => {
                let item = data.get("item").ok_or(WorkModelError::Protocol)?;
                self.item(item, out)?;
            }
            "response.incomplete" => return Err(WorkModelError::Protocol),
            "response.completed" => {
                let response = data.get("response").ok_or(WorkModelError::Protocol)?;
                self.read_usage(response);
                self.stop = Some(if self.tool_calls > 0 {
                    WorkModelStop::ToolUse
                } else if self.refused {
                    WorkModelStop::Refused
                } else {
                    WorkModelStop::EndTurn
                });
                return Ok(Flow::Done);
            }
            "response.failed" => {
                let code = data
                    .pointer("/response/error/code")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                return Err(stream_error(code));
            }
            "error" => {
                let code = data
                    .get("code")
                    .or_else(|| data.pointer("/error/code"))
                    .or_else(|| data.pointer("/error/type"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                return Err(stream_error(code));
            }
            _ => {}
        }
        Ok(Flow::Continue)
    }

    fn item(&mut self, item: &Value, out: &mut Vec<WorkModelEvent>) -> Result<(), WorkModelError> {
        match item.get("type").and_then(Value::as_str).unwrap_or_default() {
            "message" => {
                let mut text = String::new();
                for content in item
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    match content.get("type").and_then(Value::as_str) {
                        Some("output_text") => {
                            let piece = content.get("text").and_then(Value::as_str).unwrap_or("");
                            for annotation in content
                                .get("annotations")
                                .and_then(Value::as_array)
                                .into_iter()
                                .flatten()
                            {
                                if annotation.get("type").and_then(Value::as_str)
                                    != Some("url_citation")
                                {
                                    continue;
                                }
                                let url = annotation.get("url").and_then(Value::as_str);
                                let title = annotation.get("title").and_then(Value::as_str);
                                let passage = cited(piece, annotation);
                                if let Some(url) = url {
                                    self.searches.cite(url, title.unwrap_or(""), &passage);
                                }
                            }
                            push_text(&mut text, piece)?;
                        }
                        Some("refusal") => self.refused = true,
                        _ => {}
                    }
                }
                if !text.is_empty() {
                    self.assistant.push(WorkModelPart::Text(text));
                }
                self.searches.flush(out);
            }
            "function_call" => {
                if self.tool_calls >= MAX_TOOL_CALLS {
                    return Err(WorkModelError::Protocol);
                }
                let raw = item.get("arguments").and_then(Value::as_str).unwrap_or("");
                if raw.len() > MAX_TOOL_ARGUMENT_BYTES {
                    return Err(WorkModelError::Protocol);
                }
                let call = WorkModelToolCall {
                    id: item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .ok_or(WorkModelError::Protocol)?
                        .to_owned(),
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or(WorkModelError::Protocol)?
                        .to_owned(),
                    arguments: arguments(raw),
                };
                self.tool_calls += 1;
                out.push(WorkModelEvent::ToolCall(call.clone()));
                self.assistant.push(WorkModelPart::ToolCall(call));
            }
            "web_search_call" => {
                let action = item.get("action").unwrap_or(&Value::Null);
                if action.get("type").and_then(Value::as_str) == Some("search") {
                    let query = action
                        .get("query")
                        .and_then(Value::as_str)
                        .or_else(|| action.pointer("/queries/0").and_then(Value::as_str))
                        .unwrap_or("");
                    let key = item.get("id").and_then(Value::as_str).map(str::to_owned);
                    self.searches.begin(key.clone(), query);
                    for source in action
                        .get("sources")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        if let Some(url) = source.get("url").and_then(Value::as_str) {
                            let title = source.get("title").and_then(Value::as_str).unwrap_or("");
                            self.searches.hit(key.as_deref(), url, title, "");
                        }
                    }
                }
                self.assistant.push(replay(WIRE, item.clone()));
            }
            _ => self.assistant.push(replay(WIRE, item.clone())),
        }
        Ok(())
    }

    fn read_usage(&mut self, response: &Value) {
        let Some(usage) = response.get("usage") else {
            return;
        };
        self.usage.input_tokens = u64_at(usage, &["input_tokens"]);
        self.usage.cached_input_tokens = u64_at(usage, &["input_tokens_details", "cached_tokens"]);
        self.usage.output_tokens = u64_at(usage, &["output_tokens"]);
        self.usage.reasoning_tokens = u64_at(usage, &["output_tokens_details", "reasoning_tokens"]);
    }

    pub(super) fn finish(
        mut self,
        out: &mut Vec<WorkModelEvent>,
    ) -> Result<Decoded, WorkModelError> {
        let stop = self.stop.ok_or(WorkModelError::Protocol)?;
        self.searches.flush(out);
        Ok(Decoded {
            stop: DecodedStop::End(stop),
            usage: self.usage,
            searches: self.searches.count,
            assistant: self.assistant,
            ..Decoded::default()
        })
    }
}

/// The passage an annotation covers; its indices count characters.
fn cited(text: &str, annotation: &Value) -> String {
    let start = u64_at(annotation, &["start_index"]) as usize;
    let end = u64_at(annotation, &["end_index"]) as usize;
    if end <= start {
        return String::new();
    }
    let passage: String = text.chars().skip(start).take(end - start).collect();
    clip(&passage, 400)
}
