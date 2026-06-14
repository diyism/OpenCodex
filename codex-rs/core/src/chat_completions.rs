use std::collections::HashMap;

use codex_api::ApiError;
use codex_api::ByteStream;
use codex_api::HttpTransport;
use codex_api::Provider as ApiProvider;
use codex_api::ReqwestTransport;
use codex_api::SharedAuthProvider;
use codex_api::TransportError;
use codex_api::map_api_error;
use codex_login::default_client::build_reqwest_client;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::ReasoningItemReasoningSummary;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_tools::ResponsesApiNamespaceTool;
use codex_tools::ToolSpec;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use http::HeaderValue;
use http::Method;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use crate::client_common::Prompt;
use crate::client_common::ResponseEvent;
use crate::client_common::ResponseStream;

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ChatToolCall>>,
}

#[derive(Debug, Serialize)]
struct ChatToolCall {
    id: String,
    r#type: &'static str,
    function: ChatToolCallFunction,
}

#[derive(Debug, Serialize)]
struct ChatToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Default)]
struct StreamingToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveChatCompletionItem {
    Reasoning,
    Message,
}

const CHAT_COMPLETION_REASONING_ID: &str = "chatcmpl-reasoning";
const CHAT_COMPLETION_MESSAGE_ID: &str = "chatcmpl-message";

pub(crate) async fn stream_chat_completions(
    api_provider: ApiProvider,
    api_auth: SharedAuthProvider,
    prompt: &Prompt,
    model_info: &ModelInfo,
) -> Result<ResponseStream> {
    let payload = build_chat_completion_request(prompt, model_info)?;
    let mut request = api_provider
        .build_request(Method::POST, "chat/completions")
        .with_json(&payload);
    request.headers.insert(
        http::header::ACCEPT,
        HeaderValue::from_static("text/event-stream"),
    );
    let request = api_auth
        .apply_auth(request)
        .await
        .map_err(TransportError::from)
        .map_err(ApiError::Transport)
        .map_err(map_api_error)?;

    let transport = ReqwestTransport::new(build_reqwest_client());
    let response = transport
        .stream(request)
        .await
        .map_err(ApiError::Transport)
        .map_err(map_api_error)?;

    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent>>(16);
    let consumer_dropped = CancellationToken::new();
    tokio::spawn(process_chat_completion_sse(
        response.bytes,
        api_provider.stream_idle_timeout,
        tx_event,
        consumer_dropped.clone(),
    ));

    Ok(ResponseStream {
        rx_event,
        consumer_dropped,
    })
}

fn build_chat_completion_request(
    prompt: &Prompt,
    model_info: &ModelInfo,
) -> Result<ChatCompletionRequest> {
    let mut messages = Vec::new();
    if !prompt.base_instructions.text.trim().is_empty() {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: Some(prompt.base_instructions.text.clone()),
            tool_call_id: None,
            tool_calls: None,
        });
    }

    for item in prompt.get_formatted_input() {
        append_chat_message_for_response_item(&mut messages, item);
    }

    let tools = create_tools_json_for_chat_completions_api(&prompt.tools)?;
    Ok(ChatCompletionRequest {
        model: model_info.slug.clone(),
        messages,
        stream: true,
        tool_choice: (!tools.is_empty()).then_some("auto"),
        tools,
    })
}

fn append_chat_message_for_response_item(messages: &mut Vec<ChatMessage>, item: ResponseItem) {
    match item {
        ResponseItem::Message { role, content, .. } => {
            let text = content_items_to_text(&content);
            if text.trim().is_empty() {
                return;
            }
            messages.push(ChatMessage {
                role: chat_role(&role).to_string(),
                content: Some(text),
                tool_call_id: None,
                tool_calls: None,
            });
        }
        ResponseItem::FunctionCall {
            name,
            arguments,
            call_id,
            ..
        }
        | ResponseItem::CustomToolCall {
            name,
            input: arguments,
            call_id,
            ..
        } => {
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: None,
                tool_call_id: None,
                tool_calls: Some(vec![ChatToolCall {
                    id: call_id,
                    r#type: "function",
                    function: ChatToolCallFunction { name, arguments },
                }]),
            });
        }
        ResponseItem::FunctionCallOutput { call_id, output }
        | ResponseItem::CustomToolCallOutput {
            call_id, output, ..
        } => {
            messages.push(ChatMessage {
                role: "tool".to_string(),
                content: Some(function_output_to_text(&output.body)),
                tool_call_id: Some(call_id),
                tool_calls: None,
            });
        }
        ResponseItem::ToolSearchOutput {
            call_id,
            status,
            execution,
            tools,
        } => {
            let text = serde_json::json!({
                "status": status,
                "execution": execution,
                "tools": tools,
            })
            .to_string();
            messages.push(ChatMessage {
                role: "tool".to_string(),
                content: Some(text),
                tool_call_id: call_id,
                tool_calls: None,
            });
        }
        ResponseItem::Reasoning { .. }
        | ResponseItem::LocalShellCall { .. }
        | ResponseItem::ToolSearchCall { .. }
        | ResponseItem::WebSearchCall { .. }
        | ResponseItem::ImageGenerationCall { .. }
        | ResponseItem::Compaction { .. }
        | ResponseItem::CompactionTrigger
        | ResponseItem::ContextCompaction { .. }
        | ResponseItem::Other => {}
    }
}

fn chat_role(role: &str) -> &str {
    match role {
        "assistant" => "assistant",
        "system" | "developer" => "system",
        _ => "user",
    }
}

fn content_items_to_text(content: &[ContentItem]) -> String {
    content
        .iter()
        .filter_map(|item| match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                Some(text.as_str())
            }
            ContentItem::InputImage { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn function_output_to_text(output: &FunctionCallOutputBody) -> String {
    output.to_text().unwrap_or_default()
}

fn create_tools_json_for_chat_completions_api(tools: &[ToolSpec]) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for tool in tools {
        match tool {
            ToolSpec::Function(tool) => {
                out.push(serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                        "strict": tool.strict,
                    }
                }));
            }
            ToolSpec::Namespace(namespace) => {
                for tool in &namespace.tools {
                    match tool {
                        ResponsesApiNamespaceTool::Function(tool) => {
                            out.push(serde_json::json!({
                                "type": "function",
                                "function": {
                                    "name": tool.name,
                                    "description": tool.description,
                                    "parameters": tool.parameters,
                                    "strict": tool.strict,
                                }
                            }));
                        }
                    }
                }
            }
            ToolSpec::ToolSearch { .. }
            | ToolSpec::ImageGeneration { .. }
            | ToolSpec::WebSearch { .. }
            | ToolSpec::Freeform(_) => {}
        }
    }
    Ok(out)
}

async fn process_chat_completion_sse(
    bytes: ByteStream,
    idle_timeout: std::time::Duration,
    tx_event: mpsc::Sender<Result<ResponseEvent>>,
    consumer_dropped: CancellationToken,
) {
    let _ = tx_event.send(Ok(ResponseEvent::Created)).await;
    let mut stream = bytes
        .map(|res| res.map_err(|e| CodexErr::Stream(e.to_string(), None)))
        .eventsource();
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls: HashMap<usize, StreamingToolCall> = HashMap::new();
    let mut active_item: Option<ActiveChatCompletionItem> = None;

    loop {
        let event = tokio::select! {
            _ = consumer_dropped.cancelled() => return,
            event = timeout(idle_timeout, stream.next()) => event,
        };

        let sse = match event {
            Ok(Some(Ok(event))) => event,
            Ok(Some(Err(err))) => {
                let _ = tx_event
                    .send(Err(CodexErr::Stream(err.to_string(), None)))
                    .await;
                return;
            }
            Ok(None) => {
                emit_chat_completion_done(
                    &tx_event,
                    &mut text,
                    &mut reasoning,
                    &mut tool_calls,
                    &mut active_item,
                )
                .await;
                return;
            }
            Err(_) => {
                let _ = tx_event
                    .send(Err(CodexErr::Stream(
                        "idle timeout waiting for chat completions SSE".to_string(),
                        None,
                    )))
                    .await;
                return;
            }
        };

        if sse.data.trim() == "[DONE]" {
            emit_chat_completion_done(
                &tx_event,
                &mut text,
                &mut reasoning,
                &mut tool_calls,
                &mut active_item,
            )
            .await;
            return;
        }

        let Ok(chunk) = serde_json::from_str::<Value>(&sse.data) else {
            continue;
        };

        let Some(choice) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            continue;
        };

        if let Some(delta) = choice.get("delta") {
            if let Some(reasoning_delta) = delta
                .get("reasoning_content")
                .or_else(|| delta.get("reasoning"))
                .and_then(Value::as_str)
                .filter(|delta| !delta.is_empty())
            {
                if ensure_chat_reasoning_item_started(
                    &tx_event,
                    &mut active_item,
                    &mut text,
                    &mut reasoning,
                )
                .await
                .is_err()
                {
                    return;
                }
                reasoning.push_str(reasoning_delta);
                if tx_event
                    .send(Ok(ResponseEvent::ReasoningSummaryDelta {
                        delta: reasoning_delta.to_string(),
                        summary_index: 0,
                    }))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            if let Some(content) = delta.get("content").and_then(Value::as_str) {
                if !content.is_empty() {
                    if ensure_chat_message_item_started(
                        &tx_event,
                        &mut active_item,
                        &mut text,
                        &mut reasoning,
                    )
                    .await
                    .is_err()
                    {
                        return;
                    }
                    text.push_str(content);
                    if tx_event
                        .send(Ok(ResponseEvent::OutputTextDelta(content.to_string())))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
            if let Some(delta_tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for delta_tool_call in delta_tool_calls {
                    let index = delta_tool_call
                        .get("index")
                        .and_then(Value::as_u64)
                        .and_then(|value| usize::try_from(value).ok())
                        .unwrap_or(0);
                    let call = tool_calls.entry(index).or_default();
                    if let Some(id) = delta_tool_call.get("id").and_then(Value::as_str) {
                        call.id.get_or_insert_with(|| id.to_string());
                    }
                    if let Some(function) = delta_tool_call.get("function") {
                        if let Some(name) = function.get("name").and_then(Value::as_str) {
                            call.name.get_or_insert_with(|| name.to_string());
                        }
                        if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                            call.arguments.push_str(arguments);
                        }
                    }
                }
            }
        }

        if choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .is_some()
        {
            emit_chat_completion_done(
                &tx_event,
                &mut text,
                &mut reasoning,
                &mut tool_calls,
                &mut active_item,
            )
            .await;
            return;
        }
    }
}

async fn ensure_chat_reasoning_item_started(
    tx_event: &mpsc::Sender<Result<ResponseEvent>>,
    active_item: &mut Option<ActiveChatCompletionItem>,
    text: &mut String,
    reasoning: &mut String,
) -> std::result::Result<(), ()> {
    if matches!(active_item, Some(ActiveChatCompletionItem::Reasoning)) {
        return Ok(());
    }
    finish_active_chat_completion_item(tx_event, active_item, text, reasoning).await?;
    let item = ResponseItem::Reasoning {
        id: CHAT_COMPLETION_REASONING_ID.to_string(),
        summary: Vec::new(),
        content: None,
        encrypted_content: None,
    };
    tx_event
        .send(Ok(ResponseEvent::OutputItemAdded(item)))
        .await
        .map_err(|_| ())?;
    *active_item = Some(ActiveChatCompletionItem::Reasoning);
    Ok(())
}

async fn ensure_chat_message_item_started(
    tx_event: &mpsc::Sender<Result<ResponseEvent>>,
    active_item: &mut Option<ActiveChatCompletionItem>,
    text: &mut String,
    reasoning: &mut String,
) -> std::result::Result<(), ()> {
    if matches!(active_item, Some(ActiveChatCompletionItem::Message)) {
        return Ok(());
    }
    finish_active_chat_completion_item(tx_event, active_item, text, reasoning).await?;
    let item = ResponseItem::Message {
        id: Some(CHAT_COMPLETION_MESSAGE_ID.to_string()),
        role: "assistant".to_string(),
        content: Vec::new(),
        phase: None,
    };
    tx_event
        .send(Ok(ResponseEvent::OutputItemAdded(item)))
        .await
        .map_err(|_| ())?;
    *active_item = Some(ActiveChatCompletionItem::Message);
    Ok(())
}

async fn finish_active_chat_completion_item(
    tx_event: &mpsc::Sender<Result<ResponseEvent>>,
    active_item: &mut Option<ActiveChatCompletionItem>,
    text: &mut String,
    reasoning: &mut String,
) -> std::result::Result<(), ()> {
    match active_item.take() {
        Some(ActiveChatCompletionItem::Reasoning) => {
            if !reasoning.is_empty() {
                let item = ResponseItem::Reasoning {
                    id: CHAT_COMPLETION_REASONING_ID.to_string(),
                    summary: vec![ReasoningItemReasoningSummary::SummaryText {
                        text: std::mem::take(reasoning),
                    }],
                    content: None,
                    encrypted_content: None,
                };
                tx_event
                    .send(Ok(ResponseEvent::OutputItemDone(item)))
                    .await
                    .map_err(|_| ())?;
            }
        }
        Some(ActiveChatCompletionItem::Message) => {
            if !text.is_empty() {
                let item = ResponseItem::Message {
                    id: Some(CHAT_COMPLETION_MESSAGE_ID.to_string()),
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: std::mem::take(text),
                    }],
                    phase: None,
                };
                tx_event
                    .send(Ok(ResponseEvent::OutputItemDone(item)))
                    .await
                    .map_err(|_| ())?;
            }
        }
        None => {}
    }

    Ok(())
}

async fn emit_chat_completion_done(
    tx_event: &mpsc::Sender<Result<ResponseEvent>>,
    text: &mut String,
    reasoning: &mut String,
    tool_calls: &mut HashMap<usize, StreamingToolCall>,
    active_item: &mut Option<ActiveChatCompletionItem>,
) {
    if finish_active_chat_completion_item(tx_event, active_item, text, reasoning)
        .await
        .is_err()
    {
        return;
    }

    let mut calls = std::mem::take(tool_calls).into_iter().collect::<Vec<_>>();
    calls.sort_by_key(|(index, _)| *index);
    for (_, call) in calls {
        let item = ResponseItem::FunctionCall {
            id: None,
            name: call.name.unwrap_or_default(),
            namespace: None,
            arguments: call.arguments,
            call_id: call.id.unwrap_or_default(),
        };
        if tx_event
            .send(Ok(ResponseEvent::OutputItemDone(item)))
            .await
            .is_err()
        {
            return;
        }
    }

    let _ = tx_event
        .send(Ok(ResponseEvent::Completed {
            response_id: String::new(),
            token_usage: None,
            end_turn: Some(true),
        }))
        .await;
}
