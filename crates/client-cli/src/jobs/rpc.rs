//! Pinned OMP 18.18.0 rpc-ui transport. No terminal input or prompt replay.
use anyhow::{bail, ensure, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use shared::{
    JobQuestion, JobQuestionAnswer, JobQuestionItem, JobQuestionOption, JobQuestionState,
    JobRunIdentity, JOB_MAX_FRAME_BYTES,
};
use std::collections::{BTreeMap, BTreeSet};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

const LOGICAL_LIMIT: usize = 8 * 1024 * 1024;

/// Unlike `lines`, this never allocates an unbounded physical line.
pub async fn read_frame<R: AsyncBufRead + Unpin>(reader: &mut R, limit: usize) -> Result<Value> {
    let mut bytes = Vec::new();
    loop {
        let buffer = reader.fill_buf().await?;
        ensure!(
            !buffer.is_empty(),
            "RPC stream closed before a complete frame"
        );
        let count = buffer
            .iter()
            .position(|b| *b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(buffer.len());
        ensure!(
            bytes.len() + count <= limit,
            "RPC frame exceeds negotiated limit"
        );
        let complete = buffer[count - 1] == b'\n';
        bytes.extend_from_slice(&buffer[..count]);
        reader.consume(count);
        if complete {
            return serde_json::from_slice(&bytes).context("malformed RPC frame");
        }
    }
}

pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, value: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() < JOB_MAX_FRAME_BYTES,
        "outbound RPC frame too large"
    );
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

#[derive(Default)]
pub struct Decoder {
    chunks: Option<(String, usize, usize, usize, Vec<u8>)>,
}
impl Decoder {
    pub fn decode(&mut self, value: Value) -> Result<Option<Value>> {
        if value["type"] != "rpc_chunk" {
            ensure!(self.chunks.is_none(), "interrupted RPC chunk sequence");
            return Ok(Some(value));
        }
        let id = text(&value, "chunkId")?.to_owned();
        let index = number(&value, "index")? as usize;
        let count = number(&value, "count")? as usize;
        let length = number(&value, "byteLength")? as usize;
        ensure!(
            count > 0 && count <= LOGICAL_LIMIT && length <= LOGICAL_LIMIT && index < count,
            "invalid RPC chunk bounds"
        );
        if index == 0 {
            ensure!(self.chunks.is_none(), "interleaved RPC chunks");
            self.chunks = Some((id.clone(), 0, count, length, Vec::new()));
        }
        let (expected_id, expected_index, expected_count, expected_length, bytes) = self
            .chunks
            .as_mut()
            .context("RPC chunk without beginning")?;
        ensure!(
            *expected_id == id
                && *expected_index == index
                && *expected_count == count
                && *expected_length == length,
            "RPC chunk identity mismatch"
        );
        let decoded = base64::engine::general_purpose::STANDARD.decode(text(&value, "data")?)?;
        ensure!(
            bytes.len() + decoded.len() <= length,
            "RPC chunk exceeds declared length"
        );
        bytes.extend_from_slice(&decoded);
        *expected_index += 1;
        if index + 1 != count {
            return Ok(None);
        }
        let (_, _, _, length, bytes) = self.chunks.take().unwrap();
        ensure!(bytes.len() == length, "incomplete RPC chunk payload");
        Ok(Some(serde_json::from_slice(&bytes)?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptAdmission {
    NotSubmitted,
    Uncertain,
    Confirmed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub prompt: PromptAdmission,
    pub prompt_id: String,
    pub prompt_yielded: bool,
    pub settled: bool,
    pub questions: Vec<JobQuestion>,
    pub children: BTreeMap<String, String>,
    child_indices: BTreeMap<String, String>,
    active_asks: BTreeSet<String>,
    profile_ready: bool,
    ready: bool,
    negotiated: bool,
    ask_enabled: bool,
    children_enabled: bool,
    state_ready: bool,
}

pub enum Effect {
    Send(Value),
    Qualified,
    PromptConfirmed,
    Progress(String, Option<Value>),
    Child(Value),
    Question(JobQuestion),
    AnswerConfirmed(JobQuestion),
    Settled,
}

impl Session {
    pub fn new(run_id: Uuid) -> Self {
        Self {
            prompt: PromptAdmission::NotSubmitted,
            prompt_id: format!("prompt-{run_id}"),
            prompt_yielded: false,
            settled: false,
            questions: Vec::new(),
            children: BTreeMap::new(),
            child_indices: BTreeMap::new(),
            active_asks: BTreeSet::new(),
            profile_ready: false,
            ready: false,
            negotiated: false,
            ask_enabled: false,
            children_enabled: false,
            state_ready: false,
        }
    }

    pub fn qualified(&self) -> bool {
        self.profile_ready
            && self.ready
            && self.negotiated
            && self.ask_enabled
            && self.children_enabled
            && self.state_ready
    }

    /// Called only after the caller fsyncs the changed session. A lost pipe write
    /// remains Uncertain; no reconnect path invokes this again.
    pub fn reserve_prompt(&mut self, message: String) -> Result<Value> {
        ensure!(
            self.qualified() && self.prompt == PromptAdmission::NotSubmitted,
            "prompt admission cannot be replayed"
        );
        self.prompt = PromptAdmission::Uncertain;
        Ok(json!({"id":self.prompt_id,"type":"prompt","message":message}))
    }

    pub fn reserve_answer(
        &mut self,
        question_id: Uuid,
        dialog: &str,
        tool: &str,
        answers: &[JobQuestionAnswer],
    ) -> Result<Value> {
        let question = self
            .questions
            .iter_mut()
            .find(|q| q.id == question_id)
            .context("unknown question")?;
        ensure!(
            question.dialog_id == dialog && question.tool_call_id == tool,
            "stale dialog identity"
        );
        ensure!(
            question.state == JobQuestionState::Pending,
            "answer already reserved; refusing blind retransmission"
        );
        let wire = answer_payload(question, answers)?;
        question.answers = Some(answers.to_vec());
        question.state = JobQuestionState::Uncertain;
        question.version += 1;
        Ok(json!({"type":"extension_ui_response","id":dialog,"answers":wire}))
    }

    pub fn accept(&mut self, identity: &JobRunIdentity, frame: Value) -> Result<Vec<Effect>> {
        let mut effects = Vec::new();
        let kind = text(&frame, "type")?;
        match kind {
            "apas_profile_ready" => {
                ensure!(
                    !self.profile_ready
                        && !self.ready
                        && frame["version"] == 1
                        && frame["ompVersion"] == "18.18.0"
                        && frame["askTimeout"] == 0
                        && frame["isolatedSettings"] == true
                        && frame["childQuestionEscalation"] == true,
                    "unqualified native worker configuration"
                );
                self.profile_ready = true;
            }
            "ready" => {
                ensure!(
                    self.profile_ready && !self.ready,
                    "missing pinned worker profile proof"
                );
                ensure!(
                    frame["supportedProtocolVersions"]
                        .as_array()
                        .is_some_and(|v| v.contains(&json!(2))),
                    "OMP lacks lossless protocol v2"
                );
                ensure!(
                    number(&frame, "maxFrameBytes")? == JOB_MAX_FRAME_BYTES as u64
                        && number(&frame, "maxReassembledFrameBytes")? >= LOGICAL_LIMIT as u64,
                    "unsupported RPC frame limits"
                );
                self.ready = true;
                effects.push(Effect::Send(
                    json!({"id":"protocol","type":"negotiate_protocol","protocolVersion":2}),
                ));
            }
            "response" => {
                ensure!(
                    frame["success"] == true,
                    "native RPC command rejected: {}",
                    frame["command"]
                );
                match frame["id"].as_str().unwrap_or("") {
                    "protocol" => {
                        self.negotiated = true;
                        effects.push(Effect::Send(
                            json!({"id":"ask","type":"set_ask_dialog","enabled":true}),
                        ));
                        effects.push(Effect::Send(json!({"id":"children","type":"set_subagent_subscription","level":"events"})));
                        effects.push(Effect::Send(
                            json!({"id":"initial-state","type":"get_state"}),
                        ));
                    }
                    "ask" => {
                        ensure!(
                            frame["data"]["enabled"] == true,
                            "structured questions unavailable"
                        );
                        self.ask_enabled = true;
                    }
                    "children" => {
                        ensure!(
                            frame["data"]["level"] == "events",
                            "native child subscription unavailable"
                        );
                        self.children_enabled = true;
                    }
                    "initial-state" => {
                        ensure!(
                            frame["data"]["isSettled"] == true
                                && frame["data"]["hasPendingAsyncWork"] == false,
                            "worker already has pending work"
                        );
                        self.state_ready = true;
                    }
                    id if id == self.prompt_id => {
                        ensure!(
                            self.prompt == PromptAdmission::Uncertain
                                && frame["command"] == "prompt"
                                && frame["data"]["agentInvoked"] != false,
                            "unexpected prompt admission"
                        );
                        self.prompt = PromptAdmission::Confirmed;
                        effects.push(Effect::PromptConfirmed);
                    }
                    _ => {}
                }
                if self.qualified() && self.prompt == PromptAdmission::NotSubmitted {
                    effects.push(Effect::Qualified);
                }
            }
            "prompt_result" => {
                ensure!(
                    frame["id"] == self.prompt_id && self.prompt == PromptAdmission::Confirmed,
                    "uncorrelated prompt yield"
                );
                ensure!(
                    frame["status"] == "completed" && frame["agentInvoked"] == true,
                    "native prompt failed, aborted, or did not invoke the agent"
                );
                self.prompt_yielded = true;
                effects.push(Effect::Progress(
                    "Root response yielded; waiting for all native work".into(),
                    None,
                ));
                if frame["sessionSettled"] == true {
                    self.mark_settled(&mut effects)?;
                }
            }
            "session_settled"
                if self.prompt == PromptAdmission::Confirmed && self.prompt_yielded =>
            {
                self.mark_settled(&mut effects)?
            }
            "tool_execution_start" if frame["toolName"] == "ask" => {
                self.active_asks
                    .insert(text(&frame, "toolCallId")?.to_owned());
            }
            "extension_ui_request" => match frame["method"].as_str().unwrap_or("") {
                "ask" => {
                    ensure!(
                        self.qualified() && self.active_asks.len() == 1,
                        "ambiguous or delegated question path"
                    );
                    ensure!(
                        frame
                            .get("timeout")
                            .is_none_or(|v| v.is_null() || v.as_u64() == Some(0)),
                        "question timeout/default is forbidden"
                    );
                    let tool = self.active_asks.iter().next().unwrap().clone();
                    let question = parse_question(identity, &frame, &tool)?;
                    ensure!(
                        !self
                            .questions
                            .iter()
                            .any(|q| q.dialog_id == question.dialog_id || q.tool_call_id == tool),
                        "duplicate question identity"
                    );
                    self.questions.push(question.clone());
                    effects.push(Effect::Question(question));
                }
                "cancel" => bail!("native dialog cancelled before confirmed human answer"),
                "notify" | "setStatus" | "setWidget" | "setTitle" | "set_editor_text" => {}
                _ => bail!("unsupported native dialog; no default answer supplied"),
            },
            "tool_execution_end" if frame["toolName"] == "ask" => {
                let tool = text(&frame, "toolCallId")?;
                ensure!(frame["isError"] != true, "native ask tool rejected answer");
                let question = self
                    .questions
                    .iter_mut()
                    .find(|q| q.tool_call_id == tool)
                    .context("ask completion has no durable dialog")?;
                confirm_answer(question, &frame["result"]["details"])?;
                self.active_asks.remove(tool);
                effects.push(Effect::AnswerConfirmed(question.clone()));
            }
            "subagent_lifecycle" | "subagent_progress" | "subagent_event" => {
                let payload = &frame["payload"];
                let index_key = format!(
                    "{}:{}",
                    payload["parentToolCallId"].as_str().unwrap_or("root"),
                    payload["index"]
                );
                let id = if kind == "subagent_progress" {
                    self.child_indices
                        .get(&index_key)
                        .context("child progress without a stable lifecycle identity")?
                        .clone()
                } else {
                    text(payload, "id")?.to_owned()
                };
                if kind == "subagent_lifecycle" {
                    let status = text(payload, "status")?;
                    ensure!(
                        matches!(status, "started" | "completed" | "failed" | "aborted"),
                        "unknown native child lifecycle"
                    );
                    self.child_indices.insert(index_key, id.clone());
                    self.children.insert(id.clone(), status.into());
                }
                ensure!(
                    payload["event"]["type"] != "extension_ui_request",
                    "headless child attempted direct human question"
                );
                effects.push(Effect::Child(json!({"id":id,"status":payload["status"],"index":payload["index"],
                    "parent_tool_call_id":payload["parentToolCallId"],"event":payload["event"]["type"],
                    "usage":payload["progress"]["usage"],"duration_ms":payload["progress"]["durationMs"]})));
            }
            "rpc_frame_error" | "extension_error" => bail!("native protocol failure"),
            "message_end" | "tool_execution_end" | "agent_start" | "agent_end" | "turn_end" => {
                effects.push(Effect::Progress(
                    kind.replace('_', " "),
                    Some(bounded_detail(frame)),
                ))
            }
            _ => {}
        }
        Ok(effects)
    }

    fn mark_settled(&mut self, effects: &mut Vec<Effect>) -> Result<()> {
        ensure!(
            !self
                .questions
                .iter()
                .any(|q| q.state != JobQuestionState::Confirmed),
            "session settled with unresolved human decision"
        );
        ensure!(
            !self
                .children
                .values()
                .any(|s| matches!(s.as_str(), "started" | "running" | "pending" | "queued")),
            "session settled while native child remains active"
        );
        if !self.settled {
            self.settled = true;
            effects.push(Effect::Settled);
        }
        Ok(())
    }
}

fn bounded_detail(frame: Value) -> Value {
    // Never persist giant model snapshots as progress. Full state has separate
    // bounded native session storage inside the quota-controlled sandbox.
    json!({"type":frame["type"],"tool_name":frame["toolName"],"tool_call_id":frame["toolCallId"]})
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 65536)
        .with_context(|| format!("missing or invalid {key}"))
}
fn number(value: &Value, key: &str) -> Result<u64> {
    value[key]
        .as_u64()
        .with_context(|| format!("invalid {key}"))
}

fn parse_question(identity: &JobRunIdentity, frame: &Value, tool: &str) -> Result<JobQuestion> {
    let items = frame["questions"].as_array().context("missing questions")?;
    ensure!(
        !items.is_empty() && items.len() <= 64,
        "invalid dialog size"
    );
    let mut ids = BTreeSet::new();
    let mut questions = Vec::new();
    for item in items {
        let id = text(item, "id")?.to_owned();
        ensure!(ids.insert(id.clone()), "duplicate question id");
        let options = item["options"].as_array().context("missing options")?;
        ensure!(options.len() <= 128, "too many choices");
        let mut labels = BTreeSet::new();
        let options = options
            .iter()
            .enumerate()
            .map(|(index, option)| {
                let label = text(option, "label")?.to_owned();
                ensure!(
                    labels.insert(label.clone()),
                    "ambiguous duplicate option label"
                );
                Ok(JobQuestionOption {
                    id: format!("option-{index}"),
                    label,
                    description: option["description"].as_str().map(str::to_owned),
                    preview: option["preview"].as_str().map(str::to_owned),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        questions.push(JobQuestionItem {
            id,
            question: text(item, "question")?.into(),
            header: item["header"].as_str().map(str::to_owned),
            options,
            multi: item["multi"].as_bool().unwrap_or(false),
            recommended: item["recommended"].as_u64().map(|v| v as u32),
        });
    }
    Ok(JobQuestion {
        id: Uuid::new_v4(),
        run_id: identity.run_id,
        epoch: identity.epoch,
        version: 1,
        dialog_id: text(frame, "id")?.into(),
        tool_call_id: tool.into(),
        state: JobQuestionState::Pending,
        questions,
        answers: None,
        created_at: chrono::Utc::now().to_rfc3339(),
        confirmed_at: None,
    })
}

fn answer_payload(question: &JobQuestion, answers: &[JobQuestionAnswer]) -> Result<Vec<Value>> {
    ensure!(
        answers.len() == question.questions.len(),
        "all questions must be answered"
    );
    question
        .questions
        .iter()
        .zip(answers)
        .map(|(item, answer)| {
            ensure!(
                item.id == answer.id,
                "question answer order/identity changed"
            );
            let mut seen = BTreeSet::new();
            let labels = answer
                .selected_option_ids
                .iter()
                .map(|id| {
                    ensure!(seen.insert(id), "duplicate selected option");
                    Ok(item
                        .options
                        .iter()
                        .find(|o| &o.id == id)
                        .context("unknown option identity")?
                        .label
                        .clone())
                })
                .collect::<Result<Vec<_>>>()?;
            let custom = answer
                .custom_input
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty());
            ensure!(
                custom.is_none_or(|s| s.len() <= 65536),
                "custom answer too large"
            );
            ensure!(
                item.multi
                    || (labels.len() <= 1
                        && !(custom.is_some() && !labels.is_empty())
                        && (!labels.is_empty() || custom.is_some())),
                "invalid single-select answer"
            );
            let mut value = json!({"id":item.id,"selectedOptions":labels});
            if let Some(custom) = custom {
                value["customInput"] = json!(custom);
            }
            Ok(value)
        })
        .collect()
}

fn confirm_answer(question: &mut JobQuestion, details: &Value) -> Result<()> {
    ensure!(
        question.state == JobQuestionState::Uncertain
            || question.state == JobQuestionState::AnswerDelivered,
        "provider completed ask without accepted human intent"
    );
    let expected = answer_payload(
        question,
        question.answers.as_ref().context("missing answer intent")?,
    )?;
    let results = if question.questions.len() == 1 {
        vec![details.clone()]
    } else {
        details["results"]
            .as_array()
            .context("missing complete ask results")?
            .clone()
    };
    ensure!(
        expected.len() == results.len(),
        "incomplete provider confirmation"
    );
    for (index, (expected, actual)) in expected.iter().zip(results.iter()).enumerate() {
        ensure!(
            actual["timedOut"] != true,
            "provider used a timeout default"
        );
        if question.questions.len() > 1 {
            ensure!(
                actual["id"] == question.questions[index].id,
                "provider confirmed wrong question"
            );
        }
        ensure!(
            actual["selectedOptions"] == expected["selectedOptions"]
                && actual["customInput"] == expected["customInput"],
            "provider answer differs from accepted human intent"
        );
    }
    question.state = JobQuestionState::Confirmed;
    question.version += 1;
    question.confirmed_at = Some(chrono::Utc::now().to_rfc3339());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity() -> JobRunIdentity {
        JobRunIdentity {
            project_id: "p".into(),
            job_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            cluster_owner_user_id: Uuid::new_v4(),
            machine_id: Uuid::new_v4(),
            epoch: 3,
        }
    }
    fn question() -> JobQuestion {
        parse_question(&identity(), &json!({"id":"dialog","questions":[{"id":"db","question":"Database?","options":[{"label":"A"},{"label":"B"}]}]}), "tool").unwrap()
    }
    #[test]
    fn ambiguous_labels_rejected() {
        assert!(parse_question(&identity(),&json!({"id":"d","questions":[{"id":"q","question":"?","options":[{"label":"A"},{"label":"A"}]}]}),"t").is_err());
    }
    #[test]
    fn answer_is_not_confirmed_by_delivery() {
        let mut s = Session::new(Uuid::new_v4());
        let q = question();
        let id = q.id;
        s.questions.push(q);
        let answer = vec![JobQuestionAnswer {
            id: "db".into(),
            selected_option_ids: vec!["option-1".into()],
            custom_input: None,
        }];
        s.reserve_answer(id, "dialog", "tool", &answer).unwrap();
        assert_eq!(s.questions[0].state, JobQuestionState::Uncertain);
        assert!(s.reserve_answer(id, "dialog", "tool", &answer).is_err());
        assert!(confirm_answer(
            &mut s.questions[0],
            &json!({"selectedOptions":["B"],"timedOut":true})
        )
        .is_err());
        confirm_answer(&mut s.questions[0], &json!({"selectedOptions":["B"]})).unwrap();
        assert_eq!(s.questions[0].state, JobQuestionState::Confirmed);
    }
    #[test]
    fn root_yield_does_not_settle_child_work() {
        let id = identity();
        let mut s = Session::new(id.run_id);
        s.prompt = PromptAdmission::Confirmed;
        s.children.insert("child".into(), "running".into());
        s.accept(&id,json!({"type":"prompt_result","id":s.prompt_id,"status":"completed","agentInvoked":true,"sessionSettled":false})).unwrap();
        assert!(!s.settled);
        assert!(s.accept(&id, json!({"type":"session_settled"})).is_err());
    }
    #[test]
    fn native_nested_child_payload_retains_stable_identity() {
        let identity = identity();
        let mut s = Session::new(identity.run_id);
        s.accept(&identity,json!({"type":"subagent_lifecycle","payload":{"id":"Child","index":1,"parentToolCallId":"task-1","status":"started"}})).unwrap();
        let effects=s.accept(&identity,json!({"type":"subagent_progress","payload":{"index":1,"parentToolCallId":"task-1","progress":{"durationMs":2}}})).unwrap();
        assert!(matches!(&effects[0],Effect::Child(value) if value["id"]=="Child"));
        assert_eq!(s.children["Child"], "started");
    }
    #[test]
    fn uncertain_prompt_survives_serialization_and_cannot_replay() {
        let mut s = Session::new(Uuid::new_v4());
        s.prompt = PromptAdmission::Uncertain;
        let mut restored: Session =
            serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        assert!(restored.reserve_prompt("repeat".into()).is_err());
    }
    #[test]
    fn builtin_command_completion_cannot_confirm_or_settle_a_job() {
        let identity = identity();
        let mut s = Session::new(identity.run_id);
        s.prompt = PromptAdmission::Uncertain;
        assert!(s.accept(&identity,json!({"type":"response","id":s.prompt_id,"command":"prompt","success":true,"data":{"agentInvoked":false}})).is_err());
        assert_eq!(s.prompt, PromptAdmission::Uncertain);
        s.prompt = PromptAdmission::Confirmed;
        assert!(s.accept(&identity,json!({"type":"prompt_result","id":s.prompt_id,"status":"completed","agentInvoked":false,"sessionSettled":true})).is_err());
        assert!(!s.settled);
    }
    #[test]
    fn native_confirmation_must_cover_every_question_in_order() {
        let mut q=parse_question(&identity(),&json!({"id":"d","questions":[{"id":"one","question":"First?","options":[{"label":"A"}]},{"id":"two","question":"Second?","options":[{"label":"B"}]}]}),"t").unwrap();
        let answers = vec![
            JobQuestionAnswer {
                id: "one".into(),
                selected_option_ids: vec!["option-0".into()],
                custom_input: None,
            },
            JobQuestionAnswer {
                id: "two".into(),
                selected_option_ids: vec!["option-0".into()],
                custom_input: None,
            },
        ];
        assert!(answer_payload(&q, &answers[..1]).is_err());
        q.answers = Some(answers);
        q.state = JobQuestionState::Uncertain;
        assert!(confirm_answer(&mut q,&json!({"results":[{"id":"two","selectedOptions":["B"]},{"id":"one","selectedOptions":["A"]}]})).is_err());
        assert!(confirm_answer(&mut q,&json!({"results":[{"id":"one","selectedOptions":["A"]},{"id":"two","selectedOptions":["B"],"timedOut":true}]})).is_err());
        confirm_answer(&mut q,&json!({"results":[{"id":"one","selectedOptions":["A"]},{"id":"two","selectedOptions":["B"]}]})).unwrap();
        assert_eq!(q.state, JobQuestionState::Confirmed);
    }
    #[tokio::test]
    async fn oversized_line_stops_without_reading_to_eof() {
        let mut input = tokio::io::BufReader::new(&b"12345678901234567890"[..]);
        assert!(read_frame(&mut input, 8).await.is_err());
    }
    #[test]
    fn interrupted_chunks_rejected() {
        let mut d = Decoder::default();
        assert!(d.decode(json!({"type":"rpc_chunk","chunkId":"x","index":0,"count":2,"byteLength":4,"data":"e30="})).unwrap().is_none());
        assert!(d.decode(json!({"type":"session_settled"})).is_err());
    }
}
