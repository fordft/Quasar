use std::sync::Arc;

use serde_json::{json, Value};
use zephium_core::work::model::{
    WorkModelProvider, WorkModelToolCall, WorkModelUsage, WorkModelWire,
};

use super::super::continuation::AgentProviderTranscript;
use super::super::request::{
    encode_openai_body, encode_openai_continuation_body, encode_openai_extraction_payload,
    screenshot_result_item_for_test,
};
use super::super::{
    AgentProviderModelRevision, AgentProviderPricingProfile, AgentProviderPricingRevision,
    AgentProviderPricingSchedule, AgentProviderResponseRoute, AgentProviderStreamBudget,
    AgentProviderTokenRates,
};
use super::*;
use crate::{AgentProviderKind, SemanticTokenizerRevision};

const OBJECTIVE: &str = "find the release price";
const OBSERVATION: &str = "ZSEM1\ncontent=untrusted\n@a1 button Buy";

fn config() -> AgentProviderCallConfig {
    let model =
        AgentProviderModelRevision::try_new("anthropic_claude-test-5".to_owned()).expect("model");
    AgentProviderPricingSchedule::try_new(
        AgentProviderKind::OpenAiResponses,
        model.clone(),
        vec![model],
        AgentProviderResponseRoute::ModelReported,
        AgentProviderReasoningEffort::Low,
        SemanticTokenizerRevision::try_new("reported:anthropic_claude-test-5".to_owned())
            .expect("tokenizer"),
        AgentProviderPricingProfile::try_new(
            AgentProviderPricingRevision::new(1).expect("revision"),
            200_000,
        )
        .expect("profile"),
        AgentProviderTokenRates::try_new(3_000_000, 300_000, 3_750_000, 15_000_000).expect("rates"),
    )
    .expect("schedule")
    .try_provider_reported_call_config(2_048, AgentProviderStreamBudget::STANDARD)
    .expect("config")
    .restrict_to_locate_and_act()
}

fn model() -> WorkModelRef {
    WorkModelRef {
        provider: WorkModelProvider::Anthropic,
        wire: WorkModelWire::AnthropicMessages,
        model: "claude-test-5".to_owned(),
    }
}

fn identity() -> AgentProviderCallIdentity {
    AgentProviderCallIdentity {
        manifest: crate::AgentRunManifestId::from_raw(1),
        manifest_guard: [0; 32],
        call: crate::AgentModelCallId::new(7).expect("call"),
        lease: crate::AgentPlanLeaseId::from_raw(1),
        node: crate::AgentPlanNodeId::from_raw(1),
    }
}

fn texts(parts: &[WorkModelPart]) -> Vec<&str> {
    parts
        .iter()
        .map(|part| match part {
            WorkModelPart::Text(text) => text.as_str(),
            _ => panic!("text part expected"),
        })
        .collect()
}

fn observation_body(config: &AgentProviderCallConfig) -> Value {
    serde_json::from_slice(&encode_openai_body(config, OBJECTIVE, OBSERVATION).expect("body"))
        .expect("json")
}

fn bytes(body: &Value) -> Vec<u8> {
    serde_json::to_vec(body).expect("bytes")
}

/// A continuation the encoder built after one tool turn whose reply carried
/// an encrypted reasoning item before its function call.
fn continuation_body(
    config: &AgentProviderCallConfig,
    name: &str,
    arguments: &str,
    result: &str,
) -> Value {
    let replay = OpenAiResponseReplay::try_new(vec![
        OpenAiResponseReplayItem::Reasoning {
            id: "rs_1".to_owned(),
            encrypted_content: "gAAAAencrypted".to_owned(),
        },
        OpenAiResponseReplayItem::FunctionCall,
    ])
    .expect("replay");
    let call = AgentBrowserToolCall::decode_openai_with_replay(
        identity(),
        format!("fc_{name}"),
        format!("call_{name}"),
        name,
        arguments.to_owned(),
        Some(replay),
    )
    .expect("tool call");
    let transcript =
        AgentProviderTranscript::try_initial(Arc::from(OBJECTIVE), OBSERVATION.to_owned())
            .expect("transcript")
            .try_bind_for_test(call.into_continuation_parts_for_test().0, result.to_owned())
            .expect("bound transcript");
    serde_json::from_slice(&encode_openai_continuation_body(config, &transcript).expect("body"))
        .expect("json")
}

#[test]
fn an_observation_becomes_one_user_turn_under_the_fixed_instructions() {
    let config = config();
    let mut body = observation_body(&config);
    // The shape every host fact takes, as the encoders append it.
    body["input"]
        .as_array_mut()
        .expect("input")
        .push(json!({"role":"developer","content":[{"type":"input_text","text":"ZEPHIUM_HOST_ACTION_BUDGET_V1\nnative_actions_remaining=2"}]}));
    let (request, _) = translate(&config, &model(), false, &bytes(&body)).expect("translated");
    assert_eq!(request.model, model());
    assert_eq!(
        request.system[0].text,
        body["instructions"].as_str().unwrap()
    );
    assert!(request.system[1].cache);
    assert_eq!(request.max_output_tokens, 2_048);
    assert_eq!(request.reasoning, Some(WorkModelReasoning::Low));
    assert!(!request.parallel_tools && !request.native_search);
    assert_eq!(request.tool_choice, WorkModelToolChoice::Auto);
    let sent: Vec<&str> = request
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    let built: Vec<&str> = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(sent, built);
    for (tool, wire) in request.tools.iter().zip(body["tools"].as_array().unwrap()) {
        assert_eq!(tool.schema, wire["parameters"]);
        assert_eq!(tool.description, wire["description"].as_str().unwrap());
        // The browser tools are strict, so a strict-capable provider is asked
        // to hold its calls to them exactly.
        assert_eq!(tool.strict, wire["strict"] == serde_json::json!(true));
        assert!(tool.strict);
    }
    let [WorkModelMessage::User(parts)] = request.messages.as_slice() else {
        panic!("one user turn");
    };
    assert_eq!(
        texts(parts),
        [
            OBJECTIVE,
            OBSERVATION,
            "[developer]\nZEPHIUM_HOST_ACTION_BUDGET_V1\nnative_actions_remaining=2"
        ]
    );
    // The budget changes every call, so caches close before it; the key is
    // stable for the page's fixed prompt and tools.
    assert_eq!(request.volatile_tail, 1);
    assert_eq!(
        request.cache_key,
        Some(work_model_cache_key(&request.system, &request.tools))
    );
}

#[test]
fn a_continuation_replays_each_turn_and_drops_reasoning() {
    let config = config();
    for (name, arguments, result) in [
        (
            "locate",
            r#"{"semantic_query":"Buy button","scope":{"kind":"initial"}}"#,
            "ZLOC1\n@a1 button",
        ),
        (
            "read",
            r#"{"scope":{"kind":"initial"}}"#,
            "ZREAD3\nprice 12 USD",
        ),
        (
            "act",
            r#"{"actions":[{"kind":"click","target":"@a1","effect":"read","wait":{"kind":"semantic_change"},"verification":{"kind":"page_changed"},"settle_millis":1000}]}"#,
            "ZDIFF1\nchanged",
        ),
    ] {
        let body = continuation_body(&config, name, arguments, result);
        assert!(body["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "reasoning"));
        let (request, _) = translate(&config, &model(), false, &bytes(&body)).expect(name);
        let [WorkModelMessage::User(first), WorkModelMessage::Assistant(calls), WorkModelMessage::ToolResults(results)] =
            request.messages.as_slice()
        else {
            panic!("{name}: user, assistant, tool results");
        };
        assert_eq!(texts(first), [OBJECTIVE, OBSERVATION]);
        let [WorkModelPart::ToolCall(call)] = calls.as_slice() else {
            panic!("{name}: one replayed call and no reasoning");
        };
        assert_eq!(call.id, format!("call_{name}"));
        assert_eq!(call.name, name);
        assert_eq!(
            call.arguments,
            serde_json::from_str::<Value>(arguments).unwrap()
        );
        let [result_part] = results.as_slice() else {
            panic!("{name}: one result");
        };
        assert_eq!(result_part.call, format!("call_{name}"));
        assert_eq!(result_part.content, result);
        assert!(!result_part.is_error);
    }
}

#[test]
fn extraction_forces_one_answer_tool_with_the_mapping_schema() {
    let config = config();
    let schema = json!({"type":"object","properties":{"fields":{"type":"array"}},"required":["fields"],"additionalProperties":false});
    let body: Value = serde_json::from_slice(
        &encode_openai_extraction_payload(&config, OBJECTIVE, "ZEXTRACT1\nS price", &schema)
            .expect("body"),
    )
    .expect("json");
    let (request, _) = translate(&config, &model(), false, &bytes(&body)).expect("translated");
    let [answer] = request.tools.as_slice() else {
        panic!("only the answer tool");
    };
    assert_eq!(answer.name, STRUCTURED_OUTPUT_TOOL);
    assert_eq!(answer.schema, schema);
    assert_eq!(
        request.tool_choice,
        WorkModelToolChoice::Named(STRUCTURED_OUTPUT_TOOL.to_owned())
    );
    assert!(request.system[0]
        .text
        .starts_with("You are Quasar's bounded extraction mapper"));
}

#[test]
fn a_screenshot_result_reaches_only_a_vision_model_as_one_image() {
    let config = config();
    let mut body = continuation_body(&config, "screenshot", "{}", "unused");
    let png = [0x89, b'P', b'N', b'G', 1, 2, 3];
    let input = body["input"].as_array_mut().unwrap();
    input.pop();
    input.push(screenshot_result_item_for_test("call_screenshot", &png));
    assert!(matches!(
        translate(&config, &model(), false, &bytes(&body)),
        Err(AgentProviderProtocolError::Event)
    ));
    let (request, _) = translate(&config, &model(), true, &bytes(&body)).expect("vision");
    let [.., WorkModelMessage::ToolResults(results), WorkModelMessage::User(image)] =
        request.messages.as_slice()
    else {
        panic!("tool result then image");
    };
    assert_eq!(results[0].call, "call_screenshot");
    let [WorkModelPart::Image { media_type, bytes }] = image.as_slice() else {
        panic!("one image");
    };
    assert_eq!(media_type, "image/png");
    assert_eq!(bytes, &png);
}

#[test]
fn unreviewed_shapes_are_refused_before_any_call() {
    let config = config();
    let body = observation_body(&config);
    let mutations: [fn(&mut Value); 6] = [
        |body: &mut Value| body["input"][0]["role"] = json!("system"),
        |body: &mut Value| body["input"][0]["content"][0]["type"] = json!("input_file"),
        |body: &mut Value| body["tools"][0]["name"] = json!("eval"),
        |body: &mut Value| body["tool_choice"] = json!("none"),
        |body: &mut Value| {
            body["input"]
                .as_array_mut()
                .unwrap()
                .push(json!({"type":"web_search_call"}))
        },
        |body: &mut Value| {
            let first = body["tools"][0].clone();
            body["tools"].as_array_mut().unwrap().push(first);
        },
    ];
    for mutate in mutations {
        let mut changed = body.clone();
        mutate(&mut changed);
        assert!(translate(&config, &model(), true, &bytes(&changed)).is_err());
    }
}

fn reply(stop: WorkModelStop, assistant: Vec<WorkModelPart>) -> WorkModelOutcome {
    WorkModelOutcome {
        stop,
        usage: WorkModelUsage {
            input_tokens: 500,
            cached_input_tokens: 600,
            cache_write_input_tokens: 400,
            output_tokens: 20,
            reasoning_tokens: 30,
            cost_micros: None,
        },
        assistant,
    }
}

fn call(name: &str, arguments: Value) -> WorkModelPart {
    WorkModelPart::ToolCall(WorkModelToolCall {
        id: "provider id with spaces".to_owned(),
        name: name.to_owned(),
        arguments,
    })
}

fn contract() -> (AgentProviderCallConfig, ModelClientReplyContract) {
    let config = config();
    let (_, contract) =
        translate(&config, &model(), false, &bytes(&observation_body(&config))).expect("contract");
    (config, contract)
}

#[test]
fn the_first_offered_call_is_decoded_under_zephium_ids() {
    let (config, contract) = contract();
    let reply = contract
        .finish(
            identity(),
            &config,
            reply(
                WorkModelStop::ToolUse,
                vec![
                    WorkModelPart::Text("Looking.".to_owned()),
                    call(
                        "locate",
                        json!({"semantic_query":"Buy","scope":{"kind":"initial"}}),
                    ),
                    call("act", json!({})),
                ],
            ),
        )
        .expect("reply");
    assert_eq!(reply.facts.dropped_tool_calls(), 1);
    assert_eq!(reply.facts.dropped_text_bytes(), 8);
    assert!(reply.text.is_none());
    let AgentProviderStreamConclusion::Completed(completion) = reply.finished.conclusion() else {
        panic!("completed");
    };
    assert_eq!(completion.stop(), AgentProviderStopReason::ToolCalls);
    assert!(completion.tool_only_output());
    // Subsets stay within the totals the provider reported.
    let usage = completion.usage();
    assert_eq!(usage.input_tokens(), 500);
    assert_eq!(usage.cached_input_tokens(), 500);
    assert_eq!(usage.cache_write_input_tokens(), 0);
    assert_eq!(usage.reasoning_output_tokens(), 20);
    let (_, tool) = reply.finished.into_parts();
    let (correlation, proposal) = tool.expect("tool").into_continuation_parts();
    assert_eq!(proposal.kind(), AgentBrowserToolKind::Locate);
    assert_eq!(correlation.id.as_str(), "call_zephium_7");
    assert_eq!(
        correlation.provider_item_id.as_deref(),
        Some("fc_zephium_7")
    );
}

#[test]
fn a_call_the_request_did_not_offer_or_cannot_decode_fails_closed() {
    let (config, contract) = contract();
    for (name, arguments, expected) in [
        (
            "navigate",
            json!({"url":"https://example.test/"}),
            AgentProviderProtocolError::ToolCall,
        ),
        ("eval", json!({}), AgentProviderProtocolError::ToolCall),
        (
            "locate",
            json!({"semantic_query":"Buy","scope":{"kind":"initial"},"selector":"#buy"}),
            AgentProviderProtocolError::ToolContract(
                AgentBrowserToolKind::Locate,
                crate::AgentBrowserToolContractError::Arguments,
            ),
        ),
        (
            "locate",
            json!({"semantic_query":"w ".repeat(9_000),"scope":{"kind":"initial"}}),
            AgentProviderProtocolError::Limit,
        ),
    ] {
        let result = contract.finish(
            identity(),
            &config,
            reply(WorkModelStop::ToolUse, vec![call(name, arguments)]),
        );
        match (result, expected) {
            (
                Err(AgentProviderProtocolError::ToolContract(kind, _)),
                AgentProviderProtocolError::ToolContract(expected, _),
            ) => {
                assert_eq!(kind, expected)
            }
            (Err(error), expected) => assert_eq!(error, expected, "{name}"),
            (Ok(_), _) => panic!("{name} must fail closed"),
        }
    }
}

#[test]
fn text_refusals_and_output_limits_carry_no_tool() {
    let (config, contract) = contract();
    let text = contract
        .finish(
            identity(),
            &config,
            reply(
                WorkModelStop::EndTurn,
                vec![WorkModelPart::Text("Done.".to_owned())],
            ),
        )
        .expect("text");
    assert_eq!(text.text.as_ref().map(AgentProviderTextDelta::len), Some(5));
    assert!(matches!(
        text.finished.conclusion(),
        AgentProviderStreamConclusion::Completed(completion)
            if completion.stop() == AgentProviderStopReason::Completed
                && completion.stats().output_text_bytes() == 5
    ));
    let refused = contract
        .finish(
            identity(),
            &config,
            reply(
                WorkModelStop::Refused,
                vec![WorkModelPart::Text("No.".to_owned())],
            ),
        )
        .expect("refusal");
    assert!(refused.text.is_none());
    assert_eq!(refused.facts.dropped_text_bytes(), 3);
    let limited = contract
        .finish(
            identity(),
            &config,
            reply(
                WorkModelStop::MaxTokens,
                vec![call(
                    "locate",
                    json!({"semantic_query":"Buy","scope":{"kind":"initial"}}),
                )],
            ),
        )
        .expect("limit");
    assert_eq!(limited.facts.dropped_tool_calls(), 1);
    assert!(matches!(
        limited.finished.conclusion(),
        AgentProviderStreamConclusion::Completed(completion)
            if completion.stop() == AgentProviderStopReason::OutputLimit
                && completion.stats().tool_calls() == 0
    ));
}

#[test]
fn a_structured_answer_becomes_the_mapper_output_text() {
    let config = config();
    let schema = json!({"type":"object"});
    let body: Value = serde_json::from_slice(
        &encode_openai_extraction_payload(&config, OBJECTIVE, "ZEXTRACT1", &schema).expect("body"),
    )
    .expect("json");
    let (_, contract) = translate(&config, &model(), false, &bytes(&body)).expect("contract");
    let answer = json!({"schema":1,"fields":[]});
    let forced = contract
        .finish(
            identity(),
            &config,
            reply(
                WorkModelStop::ToolUse,
                vec![
                    WorkModelPart::Text("Here it is.".to_owned()),
                    call(STRUCTURED_OUTPUT_TOOL, answer.clone()),
                ],
            ),
        )
        .expect("forced");
    let text = forced.text.expect("answer text");
    assert_eq!(
        serde_json::from_str::<Value>(text.as_str()).unwrap(),
        answer
    );
    assert!(matches!(
        forced.finished.conclusion(),
        AgentProviderStreamConclusion::Completed(completion)
            if completion.stop() == AgentProviderStopReason::Completed
                && !completion.tool_only_output()
                && completion.stats().tool_calls() == 0
                && completion.stats().output_text_bytes() as usize == text.len()
    ));
    // A provider that ignores the forced tool may answer in text.
    let plain = contract
        .finish(
            identity(),
            &config,
            reply(
                WorkModelStop::EndTurn,
                vec![WorkModelPart::Text(answer.to_string())],
            ),
        )
        .expect("plain");
    assert_eq!(plain.text.expect("text").as_str(), answer.to_string());
    assert_eq!(
        contract
            .finish(
                identity(),
                &config,
                reply(WorkModelStop::ToolUse, vec![call("locate", json!({}))]),
            )
            .err(),
        Some(AgentProviderProtocolError::ToolCall)
    );
}

/// Decodes `assistant` and translates the continuation built on its tool turn.
fn continued(assistant: Vec<WorkModelPart>) -> WorkModelRequest {
    let (config, contract) = contract();
    let reply = contract
        .finish(
            identity(),
            &config,
            reply(WorkModelStop::ToolUse, assistant),
        )
        .expect("reply");
    let (_, tool) = reply.finished.into_parts();
    let (correlation, _) = tool.expect("tool").into_continuation_parts();
    let transcript =
        AgentProviderTranscript::try_initial(Arc::from(OBJECTIVE), OBSERVATION.to_owned())
            .expect("transcript")
            .try_bind_for_test(correlation, "ZLOC1\n@a1 button".to_owned())
            .expect("bound transcript");
    let body = encode_openai_continuation_body(&config, &transcript).expect("body");
    translate(&config, &model(), false, &body)
        .expect("translated")
        .0
}

#[test]
fn a_model_s_own_turn_returns_to_its_wire_under_its_own_identity() {
    let thinking = json!({"wire": "anthropic", "value": {"type": "thinking", "thinking": "Find the button.", "signature": "EqQBsig"}});
    let request = continued(vec![
        WorkModelPart::Replay(thinking.clone()),
        WorkModelPart::Text("Looking.".to_owned()),
        call(
            "locate",
            json!({"semantic_query":"Buy","scope":{"kind":"initial"}}),
        ),
    ]);
    let [_, WorkModelMessage::Assistant(turn), WorkModelMessage::ToolResults(results)] =
        request.messages.as_slice()
    else {
        panic!("user, assistant, tool results");
    };
    let [WorkModelPart::Replay(replayed), WorkModelPart::ToolCall(tool)] = turn.as_slice() else {
        panic!("the model's own part, then its call");
    };
    assert_eq!(replayed, &thinking);
    assert_eq!(tool.id, "provider id with spaces");
    assert_eq!(results[0].call, "provider id with spaces");
}

#[test]
fn a_trimmed_reply_returns_only_zephium_s_call() {
    let request = continued(vec![
        WorkModelPart::Replay(json!({"wire": "gemini", "value": [{"functionCall": {}}]})),
        call(
            "locate",
            json!({"semantic_query":"Buy","scope":{"kind":"initial"}}),
        ),
        call("act", json!({})),
    ]);
    let [_, WorkModelMessage::Assistant(turn), WorkModelMessage::ToolResults(results)] =
        request.messages.as_slice()
    else {
        panic!("user, assistant, tool results");
    };
    let [WorkModelPart::ToolCall(tool)] = turn.as_slice() else {
        panic!("only Quasar's call");
    };
    assert_eq!(tool.id, "call_zephium_7");
    assert_eq!(results[0].call, "call_zephium_7");
}

#[test]
fn a_model_turn_without_its_call_is_refused() {
    let config = config();
    let mut body = observation_body(&config);
    body["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type": "zephium_model_turn", "call_id": "toolu_1", "parts": []}));
    assert!(translate(&config, &model(), false, &bytes(&body)).is_err());
}
