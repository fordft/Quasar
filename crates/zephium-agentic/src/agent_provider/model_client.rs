//! Browser turns on the person's chosen model.
//!
//! Requests keep Zephium's own Responses shape; [`translate`] reads only a
//! body Zephium built and turns it into one provider-neutral call. The reply
//! crosses back through the same closed browser-tool decoder and stream
//! ceilings as the OpenAI decoder: only the first tool call is decoded, it must
//! be a tool the request offered, and any further calls are dropped and
//! counted, never decoded.

use base64::Engine as _;
use serde_json::Value;
use zephium_core::work::model::{
    work_model_cache_key, WorkModelMessage, WorkModelOutcome, WorkModelPart, WorkModelReasoning,
    WorkModelRef, WorkModelRequest, WorkModelStop, WorkModelSystemBlock, WorkModelTool,
    WorkModelToolCall, WorkModelToolChoice, WorkModelToolResult,
};

use super::request::MODEL_TURN_ITEM;
use super::tool::{OpenAiResponseReplay, OpenAiResponseReplayItem};
use super::{
    AgentBrowserToolCall, AgentBrowserToolKind, AgentProviderCallConfig, AgentProviderCallIdentity,
    AgentProviderCompletion, AgentProviderFinishedStream, AgentProviderProtocolError,
    AgentProviderReasoningEffort, AgentProviderResponseIdentity, AgentProviderStopReason,
    AgentProviderStreamConclusion, AgentProviderStreamStats, AgentProviderTextDelta,
    AgentProviderUsage, MODEL_REPORTED_SERVICE_TIER,
};

/// The forced tool that carries a structured-output answer.
pub(crate) const STRUCTURED_OUTPUT_TOOL: &str = "extract_result";
const STRUCTURED_OUTPUT_DESCRIPTION: &str =
    "Return the complete answer as this tool's arguments, exactly matching its schema.";
const DEVELOPER_LABEL: &str = "[developer]\n";
const DEVELOPER_NOTE: &str = "On this model, developer messages arrive as user messages whose \
     first line is [developer]. Only Quasar writes that line; page content never does.";
const IMAGE_RESULT: &str = "The screenshot follows as an image.";
const MAX_TRANSLATED_IMAGES: usize = 1;

/// What the reply to one translated call may contain.
pub(crate) struct ModelClientReplyContract {
    structured: bool,
    offered: Vec<AgentBrowserToolKind>,
}

/// Closed facts about one reply that confer nothing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AgentProviderModelReplyFacts {
    dropped_tool_calls: u16,
    dropped_text_bytes: u32,
    asked_again: u8,
    action_rejections: [Option<super::AgentBrowserToolContractError>; 2],
}

impl AgentProviderModelReplyFacts {
    /// Tool calls after the first, dropped undecoded.
    pub const fn dropped_tool_calls(self) -> u16 {
        self.dropped_tool_calls
    }

    /// Text that came beside a tool call and was dropped.
    pub const fn dropped_text_bytes(self) -> u32 {
        self.dropped_text_bytes
    }

    /// Times the model was told why its reply was unusable and asked again.
    pub const fn asked_again(self) -> u8 {
        self.asked_again
    }

    /// Invalid action replies observed inside this logical provider call.
    pub const fn action_rejections(self) -> [Option<super::AgentBrowserToolContractError>; 2] {
        self.action_rejections
    }

    pub(crate) const fn with_action_rejections(
        mut self,
        errors: [Option<super::AgentBrowserToolContractError>; 2],
    ) -> Self {
        self.action_rejections = errors;
        self
    }

    pub(crate) const fn with_asked_again(mut self, times: u8) -> Self {
        self.asked_again = times;
        self
    }
}

/// The decoded reply: a finished stream plus any text to hand the consumer.
pub(crate) struct ModelClientReply {
    pub(crate) finished: AgentProviderFinishedStream,
    pub(crate) text: Option<AgentProviderTextDelta>,
    pub(crate) facts: AgentProviderModelReplyFacts,
}

/// Turns a body Zephium built into one call on `model`. Fails closed on any
/// item Zephium never writes, so a provider never sees an unreviewed shape.
pub(crate) fn translate(
    config: &AgentProviderCallConfig,
    model: &WorkModelRef,
    vision: bool,
    body: &[u8],
) -> Result<(WorkModelRequest, ModelClientReplyContract), AgentProviderProtocolError> {
    let invalid = || AgentProviderProtocolError::Event;
    let wire: Value = serde_json::from_slice(body).map_err(|_| invalid())?;
    let instructions = wire
        .get("instructions")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let mut messages = Vec::new();
    let mut images = 0_usize;
    // The model's own turn precedes its call and lends the call its identity.
    let mut turn: Option<(String, Vec<WorkModelPart>)> = None;
    let mut provider_ids: Vec<(String, String)> = Vec::new();
    for item in wire
        .get("input")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?
    {
        let kind = item.get("type").and_then(Value::as_str);
        if turn.is_some() && kind != Some("function_call") {
            return Err(invalid());
        }
        match kind {
            Some("reasoning") => {}
            Some(MODEL_TURN_ITEM) => {
                let parts = item
                    .get("parts")
                    .and_then(Value::as_array)
                    .ok_or_else(invalid)?
                    .iter()
                    .cloned()
                    .map(WorkModelPart::Replay)
                    .collect();
                turn = Some((string(item, "call_id")?, parts));
            }
            Some("function_call") => {
                let arguments = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .ok_or_else(invalid)?;
                let call_id = string(item, "call_id")?;
                let (id, mut parts) = match turn.take() {
                    Some((provider, parts)) => {
                        provider_ids.push((call_id, provider.clone()));
                        (provider, parts)
                    }
                    None => (call_id, Vec::new()),
                };
                parts.push(WorkModelPart::ToolCall(WorkModelToolCall {
                    id,
                    name: string(item, "name")?,
                    arguments: serde_json::from_str(arguments).map_err(|_| invalid())?,
                }));
                push(&mut messages, WorkModelMessage::Assistant(parts));
            }
            Some("function_call_output") => {
                let call = string(item, "call_id")?;
                let call = provider_ids
                    .iter()
                    .find(|(ours, _)| *ours == call)
                    .map_or(call, |(_, provider)| provider.clone());
                match item.get("output") {
                    Some(Value::String(output)) => push(
                        &mut messages,
                        WorkModelMessage::ToolResults(vec![WorkModelToolResult {
                            call,
                            content: output.clone(),
                            is_error: false,
                        }]),
                    ),
                    Some(Value::Array(parts)) => {
                        let mut image = Vec::new();
                        for part in parts {
                            image.push(input_image(part, vision, &mut images)?);
                        }
                        if image.is_empty() {
                            return Err(invalid());
                        }
                        push(
                            &mut messages,
                            WorkModelMessage::ToolResults(vec![WorkModelToolResult {
                                call,
                                content: IMAGE_RESULT.to_owned(),
                                is_error: false,
                            }]),
                        );
                        push(&mut messages, WorkModelMessage::User(image));
                    }
                    _ => return Err(invalid()),
                }
            }
            None | Some("message") => {
                let role = item.get("role").and_then(Value::as_str);
                let mut parts = Vec::new();
                for content in item
                    .get("content")
                    .and_then(Value::as_array)
                    .ok_or_else(invalid)?
                {
                    match content.get("type").and_then(Value::as_str) {
                        Some("input_text") => {
                            let text = string(content, "text")?;
                            parts.push(WorkModelPart::Text(match role {
                                Some("developer") => format!("{DEVELOPER_LABEL}{text}"),
                                _ => text,
                            }));
                        }
                        Some("input_image") if role == Some("user") => {
                            parts.push(input_image(content, vision, &mut images)?);
                        }
                        Some("output_text") if role == Some("assistant") => {
                            parts.push(WorkModelPart::Text(string(content, "text")?));
                        }
                        _ => return Err(invalid()),
                    }
                }
                match role {
                    Some("user" | "developer") if !parts.is_empty() => {
                        push(&mut messages, WorkModelMessage::User(parts));
                    }
                    Some("assistant") if !parts.is_empty() => {
                        push(&mut messages, WorkModelMessage::Assistant(parts));
                    }
                    _ => return Err(invalid()),
                }
            }
            Some(_) => return Err(invalid()),
        }
    }
    if turn.is_some() {
        return Err(invalid());
    }
    let structured = wire.pointer("/text/format").map(|format| {
        (format.get("type").and_then(Value::as_str) == Some("json_schema"))
            .then(|| format.get("schema").cloned())
            .flatten()
            .ok_or_else(invalid)
    });
    let is_structured = structured.is_some();
    let (tools, offered, tool_choice) = match structured {
        Some(schema) => {
            if wire.get("tools").is_some() {
                return Err(invalid());
            }
            (
                vec![WorkModelTool {
                    name: STRUCTURED_OUTPUT_TOOL.to_owned(),
                    description: STRUCTURED_OUTPUT_DESCRIPTION.to_owned(),
                    schema: schema?,
                    strict: false,
                }],
                Vec::new(),
                WorkModelToolChoice::Named(STRUCTURED_OUTPUT_TOOL.to_owned()),
            )
        }
        None => {
            let mut tools = Vec::new();
            let mut offered = Vec::new();
            for tool in wire
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(invalid)?
            {
                let name = string(tool, "name")?;
                let kind = AgentBrowserToolKind::parse(&name).ok_or_else(invalid)?;
                if offered.contains(&kind) {
                    return Err(invalid());
                }
                offered.push(kind);
                tools.push(WorkModelTool {
                    name,
                    description: string(tool, "description")?,
                    schema: tool.get("parameters").cloned().ok_or_else(invalid)?,
                    // The browser tools are written for strict calling: a
                    // provider that holds calls to them exactly cannot send a
                    // malformed action that ends the page.
                    strict: tool.get("strict") == Some(&Value::Bool(true)),
                });
            }
            let choice = match wire.get("tool_choice").and_then(Value::as_str) {
                Some("auto") => WorkModelToolChoice::Auto,
                Some("required") => WorkModelToolChoice::Required,
                _ => return Err(invalid()),
            };
            if tools.is_empty() {
                return Err(invalid());
            }
            (tools, offered, choice)
        }
    };
    let system = vec![
        WorkModelSystemBlock {
            text: instructions.to_owned(),
            cache: false,
        },
        WorkModelSystemBlock {
            text: DEVELOPER_NOTE.to_owned(),
            cache: true,
        },
    ];
    // The host's running facts close the newest message and change on every
    // call, so the cacheable conversation ends before them.
    let volatile_tail = match messages.last() {
        Some(WorkModelMessage::User(parts)) => parts
            .iter()
            .rev()
            .take_while(|part| {
                matches!(part, WorkModelPart::Text(text) if text.starts_with(DEVELOPER_LABEL))
            })
            .count(),
        _ => 0,
    };
    let cache_key = Some(work_model_cache_key(&system, &tools));
    Ok((
        WorkModelRequest {
            image_fence: None,
            model: model.clone(),
            system,
            tools,
            messages,
            max_output_tokens: config.max_output_tokens(),
            reasoning: match config.reasoning_effort() {
                AgentProviderReasoningEffort::None => None,
                AgentProviderReasoningEffort::Low => Some(WorkModelReasoning::Low),
                AgentProviderReasoningEffort::Medium => Some(WorkModelReasoning::Medium),
                AgentProviderReasoningEffort::High => Some(WorkModelReasoning::High),
                AgentProviderReasoningEffort::XHigh => Some(WorkModelReasoning::XHigh),
                AgentProviderReasoningEffort::Max => Some(WorkModelReasoning::Max),
            },
            native_search: false,
            parallel_tools: false,
            tool_choice,
            volatile_tail,
            cache_key,
        },
        ModelClientReplyContract {
            structured: is_structured,
            offered,
        },
    ))
}

fn string(value: &Value, key: &str) -> Result<String, AgentProviderProtocolError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(AgentProviderProtocolError::Event)
}

/// Consecutive same-kind turns merge, keeping one user turn per tool result.
fn push(messages: &mut Vec<WorkModelMessage>, next: WorkModelMessage) {
    match (messages.last_mut(), next) {
        (Some(WorkModelMessage::User(parts)), WorkModelMessage::User(more)) => parts.extend(more),
        (Some(WorkModelMessage::ToolResults(results)), WorkModelMessage::ToolResults(more)) => {
            results.extend(more)
        }
        (_, next) => messages.push(next),
    }
}

fn input_image(
    part: &Value,
    vision: bool,
    images: &mut usize,
) -> Result<WorkModelPart, AgentProviderProtocolError> {
    let invalid = AgentProviderProtocolError::Event;
    if !vision || part.get("type").and_then(Value::as_str) != Some("input_image") {
        return Err(invalid);
    }
    *images += 1;
    if *images > MAX_TRANSLATED_IMAGES {
        return Err(AgentProviderProtocolError::Limit);
    }
    let url = part
        .get("image_url")
        .and_then(Value::as_str)
        .ok_or(invalid)?;
    let (media_type, data) = url
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(";base64,"))
        .ok_or(invalid)?;
    if !matches!(media_type, "image/png" | "image/jpeg" | "image/webp") {
        return Err(invalid);
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| invalid)?;
    Ok(WorkModelPart::Image {
        media_type: media_type.to_owned(),
        bytes,
    })
}

impl ModelClientReplyContract {
    pub(crate) fn action_error(
        &self,
        outcome: &WorkModelOutcome,
    ) -> Option<super::AgentBrowserToolContractError> {
        if self.structured
            || !self.offered.contains(&AgentBrowserToolKind::Act)
            || !matches!(
                outcome.stop,
                WorkModelStop::ToolUse | WorkModelStop::EndTurn
            )
        {
            return None;
        }
        let call = outcome.assistant.iter().find_map(|part| match part {
            WorkModelPart::ToolCall(call) => Some(call),
            _ => None,
        })?;
        if call.name != "act" {
            return None;
        }
        super::tool::decode_proposal(&call.name, &serde_json::to_string(&call.arguments).ok()?)
            .err()
    }

    /// When a reply cannot become a browser turn but the model could correct
    /// it (no tool call, a tool not offered, arguments that do not decode),
    /// the messages that hand it back with the exact reason, to append to
    /// the request and ask once more. None for a reply that decodes, that
    /// ran out of output or was refused, and for structured output, whose
    /// mapper has its own told-why retry.
    pub(crate) fn reask(
        &self,
        outcome: &WorkModelOutcome,
        tools: &[WorkModelTool],
    ) -> Option<Vec<WorkModelMessage>> {
        if self.structured
            || !matches!(
                outcome.stop,
                WorkModelStop::EndTurn | WorkModelStop::ToolUse
            )
        {
            return None;
        }
        let offered = self
            .offered
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let mut turn: Vec<WorkModelPart> = outcome
            .assistant
            .iter()
            .filter(|part| {
                !matches!(
                    part,
                    WorkModelPart::ToolCall(_) | WorkModelPart::Image { .. }
                )
            })
            .cloned()
            .collect();
        let first = outcome.assistant.iter().find_map(|part| match part {
            WorkModelPart::ToolCall(call) => Some(call),
            _ => None,
        });
        let mut messages = Vec::new();
        let Some(call) = first else {
            if !turn.is_empty() {
                messages.push(WorkModelMessage::Assistant(turn));
            }
            messages.push(WorkModelMessage::User(vec![WorkModelPart::Text(format!(
                "{DEVELOPER_LABEL}no_tool_call: nothing ran. Every reply on this page must be exactly one call to one of the offered tools ({offered}); text alone is not read. Call one now."
            ))]));
            return Some(messages);
        };
        let problem = match AgentBrowserToolKind::parse(&call.name)
            .filter(|kind| self.offered.contains(kind))
        {
            None => {
                let name: String = call.name.chars().take(64).collect();
                format!(
                    "invalid_tool_call: nothing ran. `{name}` is not a tool offered on this turn. Call exactly one of: {offered}."
                )
            }
            Some(_) => {
                let arguments = serde_json::to_string(&call.arguments).ok()?;
                let schema = tools
                    .iter()
                    .find(|tool| tool.name == call.name)
                    .map(|tool| &tool.schema);
                super::tool::browser_call_problem(&call.name, &arguments, schema)?
            }
        };
        turn.push(WorkModelPart::ToolCall(call.clone()));
        messages.push(WorkModelMessage::Assistant(turn));
        messages.push(WorkModelMessage::ToolResults(vec![WorkModelToolResult {
            call: call.id.clone(),
            content: problem,
            is_error: true,
        }]));
        Some(messages)
    }

    /// A browser reply that ran past its output limit is asked once more
    /// for one tool call alone. Its runaway text is not handed back.
    pub(crate) fn reask_runaway(
        &self,
        outcome: &WorkModelOutcome,
    ) -> Option<Vec<WorkModelMessage>> {
        if self.structured || outcome.stop != WorkModelStop::MaxTokens {
            return None;
        }
        let offered = self
            .offered
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Some(vec![WorkModelMessage::User(vec![WorkModelPart::Text(format!(
            "{DEVELOPER_LABEL}output_limit: nothing ran. Your reply ran past its length limit. Be brief: reply now with exactly one call to one of the offered tools ({offered}), no other text and no long reasoning."
        ))])])
    }

    /// Whether this call answers in structured output rather than a tool.
    pub(crate) const fn is_structured(&self) -> bool {
        self.structured
    }

    /// Decodes the reply through the closed browser-tool contract. Usage is
    /// the provider's; the identity is the requested model, never a claim the
    /// provider made.
    pub(crate) fn finish(
        &self,
        call: AgentProviderCallIdentity,
        config: &AgentProviderCallConfig,
        outcome: WorkModelOutcome,
    ) -> Result<ModelClientReply, AgentProviderProtocolError> {
        let budget = config.stream_budget();
        let reported = outcome.usage;
        let cached = reported.cached_input_tokens.min(reported.input_tokens);
        let usage = AgentProviderUsage::try_new(
            reported.input_tokens,
            reported.output_tokens,
            cached,
            reported
                .cache_write_input_tokens
                .min(reported.input_tokens - cached),
            reported.reasoning_tokens.min(reported.output_tokens),
        )
        .map_err(|_| AgentProviderProtocolError::Usage)?;
        let identity = AgentProviderResponseIdentity::try_attested(
            config,
            config.model().as_str(),
            MODEL_REPORTED_SERVICE_TIER,
            None,
        )?;
        let mut text = String::new();
        let mut calls = Vec::new();
        let mut replays = Vec::new();
        for part in outcome.assistant {
            match part {
                WorkModelPart::Text(piece) => {
                    if text.len().saturating_add(piece.len())
                        > crate::MAX_AGENT_PROVIDER_REQUEST_BYTES
                    {
                        return Err(AgentProviderProtocolError::Limit);
                    }
                    text.push_str(&piece);
                }
                WorkModelPart::ToolCall(tool) => calls.push(tool),
                WorkModelPart::Image { .. } => {
                    return Err(AgentProviderProtocolError::UnsupportedOutput)
                }
                WorkModelPart::Replay(value) => replays.push(value),
            }
        }
        let mut facts = AgentProviderModelReplyFacts {
            dropped_tool_calls: u16::try_from(calls.len().saturating_sub(1)).unwrap_or(u16::MAX),
            dropped_text_bytes: 0,
            asked_again: 0,
            action_rejections: [None; 2],
        };
        let first = calls.into_iter().next();
        let bounded = |bytes: usize, limit: u32| {
            u32::try_from(bytes)
                .ok()
                .filter(|bytes| *bytes <= limit)
                .ok_or(AgentProviderProtocolError::Limit)
        };
        let completion = |stop, stats, tool_only| {
            AgentProviderStreamConclusion::Completed(
                AgentProviderCompletion::new_with_response_identity(
                    call, stop, usage, stats, tool_only, identity,
                ),
            )
        };
        let text_reply = |stop, text: String, facts| {
            let bytes = bounded(text.len(), budget.max_output_text_bytes())?;
            let stats = AgentProviderStreamStats::new(bytes, 1, bytes, 0, 0);
            Ok(ModelClientReply {
                finished: AgentProviderFinishedStream::new(completion(stop, stats, false), None)?,
                text: (!text.is_empty()).then(|| AgentProviderTextDelta::new(text)),
                facts,
            })
        };
        match outcome.stop {
            WorkModelStop::Refused => {
                facts.dropped_text_bytes = u32::try_from(text.len()).unwrap_or(u32::MAX);
                text_reply(AgentProviderStopReason::Refused, String::new(), facts)
            }
            WorkModelStop::MaxTokens => {
                facts.dropped_tool_calls = facts
                    .dropped_tool_calls
                    .saturating_add(u16::from(first.is_some()));
                text_reply(AgentProviderStopReason::OutputLimit, text, facts)
            }
            WorkModelStop::EndTurn | WorkModelStop::ToolUse if self.structured => {
                // A provider that ignores the forced tool may still answer in
                // text; the extraction mapper validates either strictly.
                let answer = match first {
                    Some(tool) if tool.name == STRUCTURED_OUTPUT_TOOL => {
                        facts.dropped_text_bytes = u32::try_from(text.len()).unwrap_or(u32::MAX);
                        serde_json::to_string(&tool.arguments)
                            .map_err(|_| AgentProviderProtocolError::ToolCall)?
                    }
                    Some(_) => return Err(AgentProviderProtocolError::ToolCall),
                    None => text,
                };
                text_reply(AgentProviderStopReason::Completed, answer, facts)
            }
            WorkModelStop::EndTurn | WorkModelStop::ToolUse => {
                let Some(tool) = first else {
                    return text_reply(AgentProviderStopReason::Completed, text, facts);
                };
                facts.dropped_text_bytes = u32::try_from(text.len()).unwrap_or(u32::MAX);
                let kind = AgentBrowserToolKind::parse(&tool.name)
                    .filter(|kind| self.offered.contains(kind))
                    .ok_or(AgentProviderProtocolError::ToolCall)?;
                let arguments = serde_json::to_string(&tool.arguments)
                    .map_err(|_| AgentProviderProtocolError::ToolCall)?;
                let argument_bytes = bounded(arguments.len(), budget.max_tool_argument_bytes())?;
                // Tool-call identities are Zephium's, one per model call. The
                // provider's id and own parts ride along only to be given back
                // to its wire, and only when its one call was kept whole.
                let ordinal = call.call().get();
                let function_call = || {
                    OpenAiResponseReplay::try_new(vec![OpenAiResponseReplayItem::FunctionCall])
                        .ok_or(AgentProviderProtocolError::Sequence)
                };
                let replay = if facts.dropped_tool_calls == 0 {
                    OpenAiResponseReplay::try_new(vec![
                        OpenAiResponseReplayItem::ModelTurn {
                            call_id: tool.id,
                            parts: replays,
                        },
                        OpenAiResponseReplayItem::FunctionCall,
                    ])
                    .map_or_else(function_call, Ok)?
                } else {
                    function_call()?
                };
                let decoded = AgentBrowserToolCall::decode_openai_with_replay(
                    call,
                    format!("fc_zephium_{ordinal}"),
                    format!("call_zephium_{ordinal}"),
                    kind.as_str(),
                    arguments,
                    Some(replay),
                )
                .map_err(|error| AgentProviderProtocolError::ToolContract(kind, error))?;
                let stats = AgentProviderStreamStats::new(argument_bytes, 1, 0, 1, argument_bytes);
                Ok(ModelClientReply {
                    finished: AgentProviderFinishedStream::new(
                        completion(AgentProviderStopReason::ToolCalls, stats, true),
                        Some(decoded),
                    )?,
                    text: None,
                    facts,
                })
            }
        }
    }
}

#[cfg(test)]
#[path = "model_client_tests.rs"]
mod tests;
