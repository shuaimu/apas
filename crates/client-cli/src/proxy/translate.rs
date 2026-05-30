//! Pure translation between OpenAI Responses API and DeepSeek's
//! Chat Completions API (which speaks the OpenAI chat shape).
//!
//! We keep the types loose (lots of `serde_json::Value`) where the
//! upstream/downstream is wide and unstable; only the well-known
//! framing fields get strong types. That way schema drift on either
//! side doesn't break compilation — it surfaces in the integration
//! tests instead, where it's easier to chase.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

// -------- Responses request (codex -> us) ----------

/// Subset of OpenAI Responses API request fields codex sends.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ResponsesRequest {
    pub model: Option<String>,
    /// System prompt. Codex sends instructions either as a plain
    /// string or, more rarely, as a content-array; we accept both.
    pub instructions: Option<Value>,
    /// Either a plain string (`"hi"`) or an array of structured
    /// items (`{"type": "message" | "function_call" | …}`).
    pub input: Option<Value>,
    /// `[{"type": "function", "name": "...", "parameters": {...}, ...}]`
    pub tools: Option<Vec<Value>>,
    pub tool_choice: Option<Value>,
    pub stream: Option<bool>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_output_tokens: Option<u64>,
}

// -------- Chat Completions request (us -> DeepSeek) ----------

#[derive(Debug, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ChatMessage {
    pub role: String,
    /// Text body. `None` when the message is a tool-call assistant
    /// turn (the chat-completions wire form expects `content: null`
    /// alongside `tool_calls`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<Value>,
}

// -------- Translation: request ----------

/// Translate a Codex Responses request into a DeepSeek Chat
/// Completions request.
pub fn responses_to_chat(req: &ResponsesRequest) -> ChatRequest {
    let mut messages: Vec<ChatMessage> = Vec::new();

    if let Some(system) = extract_text(req.instructions.as_ref()) {
        if !system.is_empty() {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: Some(system),
                tool_call_id: None,
                tool_calls: Vec::new(),
            });
        }
    }

    match &req.input {
        Some(Value::String(text)) => {
            messages.push(ChatMessage {
                role: "user".to_string(),
                content: Some(text.clone()),
                tool_call_id: None,
                tool_calls: Vec::new(),
            });
        }
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(msg) = item_to_chat_message(item) {
                    coalesce_assistant_tool_calls(&mut messages, msg);
                }
            }
        }
        _ => {}
    }

    ChatRequest {
        model: req.model.clone().unwrap_or_else(|| "deepseek-chat".to_string()),
        messages,
        tools: req
            .tools
            .as_ref()
            .map(|tools| tools.iter().map(responses_tool_to_chat).collect())
            .unwrap_or_default(),
        tool_choice: req.tool_choice.clone(),
        stream: req.stream.unwrap_or(false),
        temperature: req.temperature,
        top_p: req.top_p,
        max_tokens: req.max_output_tokens,
    }
}

/// Pull text out of an `instructions` field that may be a string or a
/// content-block array (`[{"type": "input_text", "text": "..."}, ...]`).
fn extract_text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => {
            let mut buf = String::new();
            for item in items {
                if let Some(t) = item.get("text").and_then(Value::as_str) {
                    if !buf.is_empty() {
                        buf.push('\n');
                    }
                    buf.push_str(t);
                }
            }
            Some(buf)
        }
        _ => None,
    }
}

/// Convert one entry from the Responses `input` array into a chat
/// message. Returns None for shapes we don't recognize so an unknown
/// item doesn't poison the whole turn.
fn item_to_chat_message(item: &Value) -> Option<ChatMessage> {
    let kind = item.get("type").and_then(Value::as_str).unwrap_or("message");
    match kind {
        "message" => {
            let role = item
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user")
                .to_string();
            // Chat expects developer/system → "system".
            let role = if role == "developer" {
                "system".to_string()
            } else {
                role
            };
            let content = item
                .get("content")
                .map(content_blocks_to_text)
                .unwrap_or_default();
            Some(ChatMessage {
                role,
                content: Some(content),
                tool_call_id: None,
                tool_calls: Vec::new(),
            })
        }
        "function_call" => {
            // Codex emits `{name, arguments, call_id}` as a top-level
            // item. Chat wants this nested inside an assistant
            // message's `tool_calls`.
            let call_id = item
                .get("call_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let arguments = item
                .get("arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}")
                .to_string();
            let tool_call = json!({
                "id": call_id,
                "type": "function",
                "function": { "name": name, "arguments": arguments },
            });
            Some(ChatMessage {
                role: "assistant".to_string(),
                content: None,
                tool_call_id: None,
                tool_calls: vec![tool_call],
            })
        }
        "function_call_output" => {
            let call_id = item
                .get("call_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            // `output` is sometimes a string, sometimes a structured
            // object. Stringify objects so chat-completions sees a
            // single string body.
            let output = match item.get("output") {
                Some(Value::String(s)) => s.clone(),
                Some(v) => v.to_string(),
                None => String::new(),
            };
            Some(ChatMessage {
                role: "tool".to_string(),
                content: Some(output),
                tool_call_id: Some(call_id),
                tool_calls: Vec::new(),
            })
        }
        _ => None,
    }
}

/// Flatten a content-block array into a single string. Recognized
/// kinds: `input_text`, `output_text`. Everything else falls through
/// silently — image/audio inputs aren't supported by DeepSeek-chat
/// anyway.
fn content_blocks_to_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(items) => {
            let mut buf = String::new();
            for item in items {
                let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
                if matches!(kind, "input_text" | "output_text" | "text") {
                    if let Some(t) = item.get("text").and_then(Value::as_str) {
                        if !buf.is_empty() {
                            buf.push('\n');
                        }
                        buf.push_str(t);
                    }
                }
            }
            buf
        }
        _ => String::new(),
    }
}

/// Translate a Responses tool definition to the Chat Completions
/// `{type: "function", function: {...}}` envelope.
fn responses_tool_to_chat(tool: &Value) -> Value {
    let obj = match tool.as_object() {
        Some(o) => o,
        None => return tool.clone(),
    };
    let mut function = Map::new();
    if let Some(name) = obj.get("name") {
        function.insert("name".to_string(), name.clone());
    }
    if let Some(desc) = obj.get("description") {
        function.insert("description".to_string(), desc.clone());
    }
    if let Some(params) = obj.get("parameters") {
        function.insert("parameters".to_string(), params.clone());
    }
    if let Some(strict) = obj.get("strict") {
        function.insert("strict".to_string(), strict.clone());
    }
    json!({ "type": "function", "function": Value::Object(function) })
}

/// If `msg` is an assistant tool_call and the previous message is
/// also assistant tool_calls, merge — chat completions wants one
/// assistant turn with all tool_calls listed.
fn coalesce_assistant_tool_calls(messages: &mut Vec<ChatMessage>, msg: ChatMessage) {
    if msg.role == "assistant" && !msg.tool_calls.is_empty() {
        if let Some(last) = messages.last_mut() {
            if last.role == "assistant" && last.content.is_none() {
                last.tool_calls.extend(msg.tool_calls);
                return;
            }
        }
    }
    messages.push(msg);
}

// -------- Translation: response (non-streaming) ----------

/// DeepSeek chat-completion response → Codex Responses response.
///
/// `chat` is the raw JSON body DeepSeek returned. We produce the
/// Responses-shape JSON codex expects.
pub fn chat_response_to_responses(chat: &Value, model_hint: &str) -> Value {
    let id = chat
        .get("id")
        .and_then(Value::as_str)
        .map(|s| format!("resp_{}", s))
        .unwrap_or_else(|| format!("resp_{}", uuid::Uuid::new_v4()));
    let model = chat
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(model_hint)
        .to_string();

    let mut output: Vec<Value> = Vec::new();

    if let Some(choices) = chat.get("choices").and_then(Value::as_array) {
        for choice in choices {
            let message = match choice.get("message") {
                Some(m) => m,
                None => continue,
            };
            // Text body → an output_text content block under a message item.
            if let Some(text) = message.get("content").and_then(Value::as_str) {
                if !text.is_empty() {
                    output.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "status": "completed",
                        "content": [
                            { "type": "output_text", "text": text }
                        ],
                    }));
                }
            }
            // Tool calls → one function_call item per call.
            if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    let id = call.get("id").and_then(Value::as_str).unwrap_or_default();
                    let fcn = call.get("function");
                    let name = fcn
                        .and_then(|f| f.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let args = fcn
                        .and_then(|f| f.get("arguments"))
                        .and_then(Value::as_str)
                        .unwrap_or("{}");
                    output.push(json!({
                        "type": "function_call",
                        "name": name,
                        "arguments": args,
                        "call_id": id,
                        "status": "completed",
                    }));
                }
            }
        }
    }

    let usage = chat.get("usage").map(translate_usage);

    let mut resp = json!({
        "id": id,
        "object": "response",
        "model": model,
        "status": "completed",
        "output": output,
    });
    if let Some(u) = usage {
        resp.as_object_mut().unwrap().insert("usage".into(), u);
    }
    resp
}

/// Translate chat-completions usage to Responses usage names.
fn translate_usage(u: &Value) -> Value {
    let prompt = u.get("prompt_tokens").cloned().unwrap_or(json!(0));
    let completion = u.get("completion_tokens").cloned().unwrap_or(json!(0));
    let total = u.get("total_tokens").cloned().unwrap_or(json!(0));
    json!({
        "input_tokens": prompt,
        "output_tokens": completion,
        "total_tokens": total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse_req(v: Value) -> ResponsesRequest {
        serde_json::from_value(v).expect("parse responses request")
    }

    #[test]
    fn plain_text_input_becomes_user_message() {
        let req = parse_req(json!({
            "model": "deepseek-chat",
            "input": "hello",
        }));
        let chat = responses_to_chat(&req);
        assert_eq!(chat.model, "deepseek-chat");
        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].role, "user");
        assert_eq!(chat.messages[0].content.as_deref(), Some("hello"));
    }

    #[test]
    fn instructions_prepend_a_system_message() {
        let req = parse_req(json!({
            "model": "deepseek-chat",
            "instructions": "be brief",
            "input": "hi",
        }));
        let chat = responses_to_chat(&req);
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[0].role, "system");
        assert_eq!(chat.messages[0].content.as_deref(), Some("be brief"));
        assert_eq!(chat.messages[1].role, "user");
    }

    #[test]
    fn structured_user_message_with_input_text_flattens() {
        let req = parse_req(json!({
            "model": "deepseek-chat",
            "input": [
                { "type": "message", "role": "user", "content": [
                    { "type": "input_text", "text": "what's up?" }
                ]}
            ],
        }));
        let chat = responses_to_chat(&req);
        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].role, "user");
        assert_eq!(chat.messages[0].content.as_deref(), Some("what's up?"));
    }

    #[test]
    fn developer_role_maps_to_system() {
        let req = parse_req(json!({
            "input": [
                { "type": "message", "role": "developer", "content": [
                    { "type": "input_text", "text": "follow these rules" }
                ]}
            ],
        }));
        let chat = responses_to_chat(&req);
        assert_eq!(chat.messages[0].role, "system");
    }

    #[test]
    fn function_call_history_lifts_into_assistant_tool_calls() {
        let req = parse_req(json!({
            "input": [
                { "type": "message", "role": "user", "content": [
                    { "type": "input_text", "text": "ls" }
                ]},
                { "type": "function_call", "name": "shell",
                  "arguments": "{\"cmd\":\"ls\"}", "call_id": "call_1" },
                { "type": "function_call_output", "call_id": "call_1",
                  "output": "a.txt\nb.txt" },
            ],
        }));
        let chat = responses_to_chat(&req);
        assert_eq!(chat.messages.len(), 3);
        assert_eq!(chat.messages[1].role, "assistant");
        assert_eq!(chat.messages[1].content, None);
        assert_eq!(chat.messages[1].tool_calls.len(), 1);
        assert_eq!(
            chat.messages[1].tool_calls[0]["function"]["name"],
            json!("shell")
        );
        assert_eq!(chat.messages[2].role, "tool");
        assert_eq!(chat.messages[2].tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(chat.messages[2].content.as_deref(), Some("a.txt\nb.txt"));
    }

    #[test]
    fn consecutive_function_calls_merge_into_one_assistant_turn() {
        let req = parse_req(json!({
            "input": [
                { "type": "function_call", "name": "a", "arguments": "{}",
                  "call_id": "c1" },
                { "type": "function_call", "name": "b", "arguments": "{}",
                  "call_id": "c2" },
            ],
        }));
        let chat = responses_to_chat(&req);
        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].role, "assistant");
        assert_eq!(chat.messages[0].tool_calls.len(), 2);
    }

    #[test]
    fn responses_tool_def_lifts_into_function_envelope() {
        let req = parse_req(json!({
            "input": "go",
            "tools": [
                {
                    "type": "function",
                    "name": "shell",
                    "description": "run shell",
                    "parameters": { "type": "object" },
                    "strict": true
                }
            ],
        }));
        let chat = responses_to_chat(&req);
        assert_eq!(chat.tools.len(), 1);
        assert_eq!(chat.tools[0]["type"], json!("function"));
        assert_eq!(chat.tools[0]["function"]["name"], json!("shell"));
        assert_eq!(chat.tools[0]["function"]["description"], json!("run shell"));
        assert_eq!(chat.tools[0]["function"]["strict"], json!(true));
    }

    #[test]
    fn max_output_tokens_renames_to_max_tokens() {
        let req = parse_req(json!({
            "input": "x",
            "max_output_tokens": 512,
            "temperature": 0.2,
            "top_p": 0.9,
        }));
        let chat = responses_to_chat(&req);
        assert_eq!(chat.max_tokens, Some(512));
        assert_eq!(chat.temperature, Some(0.2));
        assert_eq!(chat.top_p, Some(0.9));
    }

    #[test]
    fn stream_flag_passes_through() {
        let req = parse_req(json!({"input": "x", "stream": true}));
        let chat = responses_to_chat(&req);
        assert!(chat.stream);
        let req2 = parse_req(json!({"input": "x"}));
        assert!(!responses_to_chat(&req2).stream);
    }

    #[test]
    fn chat_response_text_lifts_into_output_message() {
        let chat = json!({
            "id": "chatcmpl-1",
            "model": "deepseek-chat",
            "choices": [{
                "message": { "role": "assistant", "content": "hi back" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 5, "completion_tokens": 7, "total_tokens": 12 }
        });
        let r = chat_response_to_responses(&chat, "deepseek-chat");
        assert_eq!(r["object"], json!("response"));
        assert_eq!(r["status"], json!("completed"));
        assert!(r["id"].as_str().unwrap().starts_with("resp_"));
        assert_eq!(r["model"], json!("deepseek-chat"));
        let output = r["output"].as_array().unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0]["type"], json!("message"));
        assert_eq!(output[0]["role"], json!("assistant"));
        assert_eq!(output[0]["content"][0]["type"], json!("output_text"));
        assert_eq!(output[0]["content"][0]["text"], json!("hi back"));
        assert_eq!(r["usage"]["input_tokens"], json!(5));
        assert_eq!(r["usage"]["output_tokens"], json!(7));
    }

    #[test]
    fn chat_response_tool_calls_lift_into_function_call_items() {
        let chat = json!({
            "id": "chatcmpl-2",
            "model": "deepseek-chat",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call_a",
                            "type": "function",
                            "function": { "name": "shell", "arguments": "{\"cmd\":\"ls\"}" }
                        }
                    ]
                },
                "finish_reason": "tool_calls"
            }]
        });
        let r = chat_response_to_responses(&chat, "deepseek-chat");
        let output = r["output"].as_array().unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0]["type"], json!("function_call"));
        assert_eq!(output[0]["name"], json!("shell"));
        assert_eq!(output[0]["call_id"], json!("call_a"));
        assert_eq!(output[0]["arguments"], json!("{\"cmd\":\"ls\"}"));
    }

    #[test]
    fn chat_response_text_and_tool_calls_both_appear() {
        let chat = json!({
            "id": "chatcmpl-3",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "let me check",
                    "tool_calls": [
                        { "id": "c1", "function": { "name": "shell", "arguments": "{}" } }
                    ]
                }
            }]
        });
        let r = chat_response_to_responses(&chat, "deepseek-chat");
        let output = r["output"].as_array().unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0]["type"], json!("message"));
        assert_eq!(output[1]["type"], json!("function_call"));
    }
}
