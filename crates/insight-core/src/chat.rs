//! App-owned chat transcript items and the stream that builds them.
//!
//! Stored history and a live agent run are both normalized into
//! [`TranscriptItem`]s here. [`from_messages`] renders stored history and
//! [`RunTranscript`] folds rig's stream into the same items, keyed so a
//! committed run keeps the items the live fold emitted.

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use rig::agent::MultiTurnStreamItem;
use rig::completion::{AssistantContent, Message};
use rig::message::{ToolCall, ToolResult, ToolResultContent, UserContent};
use rig::streaming::{Item, PartKind, StreamEvent};
use serde::Serialize;

/// The kind of block a [`TranscriptItem`] carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptKind {
    /// Text written by the user.
    UserText,
    /// Text written by the assistant.
    AssistantText,
    /// Reasoning the assistant showed.
    Reasoning,
    /// A tool call the assistant requested.
    ToolCall,
    /// A tool result answering a [`TranscriptKind::ToolCall`].
    ToolResult,
}

/// One block of a conversation transcript.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TranscriptItem {
    /// Stable identity of the block, unique within its conversation.
    pub key: String,
    /// What the block holds.
    pub kind: TranscriptKind,
    /// Tool name for [`TranscriptKind::ToolCall`] and
    /// [`TranscriptKind::ToolResult`]; empty otherwise.
    pub name: String,
    /// The block's body: text, reasoning, tool-call arguments, or tool output.
    /// Empty for a tool call whose arguments have not streamed yet.
    pub text: String,
    /// Whether the item belongs to a run that has not committed yet.
    pub provisional: bool,
    /// Whether a tool result reports a failed tool call.
    pub is_error: bool,
    /// Whether a committed tool call has no result yet.
    pub executing: bool,
    /// Whether an assistant text item is currently receiving deltas.
    pub streaming: bool,
}

/// One change to a conversation transcript.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    /// Add an item, or replace the item with the same key.
    ItemUpserted {
        /// The item to add or replace.
        item: TranscriptItem,
    },
    /// Remove the item with this key.
    ItemRemoved {
        /// Key of the removed item.
        key: String,
    },
    /// The run committed: the transcript's authoritative items.
    RunFinished {
        /// Every item the run recorded, with provisional and live flags
        /// cleared.
        items: Vec<TranscriptItem>,
    },
    /// The run failed before committing.
    RunFailed {
        /// The failure, safe to show to the user.
        message: String,
    },
}

/// Normalize stored messages into transcript items.
///
/// Keys are `{base + message_index}-{block_index}`, where `block_index`
/// counts every content block of the message, including blocks that render
/// nothing. User text, tool results, assistant text, reasoning, and tool calls
/// render; other block kinds are skipped.
pub fn from_messages(messages: &[Message], base: usize) -> Vec<TranscriptItem> {
    let mut items = Vec::new();
    for (message_index, message) in messages.iter().enumerate() {
        let message_key = base + message_index;
        match message {
            Message::User { content } => {
                for (block_index, block) in content.iter().enumerate() {
                    let item = match block {
                        UserContent::Text(text) => TranscriptItem {
                            key: block_key(message_key, block_index),
                            kind: TranscriptKind::UserText,
                            name: String::new(),
                            text: text.text.clone(),
                            provisional: false,
                            is_error: false,
                            executing: false,
                            streaming: false,
                        },
                        UserContent::ToolResult(result) => TranscriptItem {
                            key: block_key(message_key, block_index),
                            kind: TranscriptKind::ToolResult,
                            name: result.name.to_string(),
                            text: tool_result_text(&result.content),
                            provisional: false,
                            is_error: result.is_error,
                            executing: false,
                            streaming: false,
                        },
                        _ => continue,
                    };
                    items.push(item);
                }
            }
            Message::Assistant(message) => {
                for (block_index, block) in message.content.iter().enumerate() {
                    let item = match block {
                        AssistantContent::Text(text) => TranscriptItem {
                            key: block_key(message_key, block_index),
                            kind: TranscriptKind::AssistantText,
                            name: String::new(),
                            text: text.text.clone(),
                            provisional: false,
                            is_error: false,
                            executing: false,
                            streaming: false,
                        },
                        AssistantContent::Reasoning(reasoning) => TranscriptItem {
                            key: block_key(message_key, block_index),
                            kind: TranscriptKind::Reasoning,
                            name: String::new(),
                            text: reasoning.text.clone(),
                            provisional: false,
                            is_error: false,
                            executing: false,
                            streaming: false,
                        },
                        AssistantContent::ToolCall(call) => TranscriptItem {
                            key: block_key(message_key, block_index),
                            kind: TranscriptKind::ToolCall,
                            name: call.function.name.to_string(),
                            text: tool_call_text(call),
                            provisional: false,
                            is_error: false,
                            executing: false,
                            streaming: false,
                        },
                        _ => continue,
                    };
                    items.push(item);
                }
            }
            Message::System { .. } => {}
        }
    }
    items
}

/// Fold one agent run's stream into transcript events.
///
/// `base` is the number of messages already stored when the run started, so
/// live keys are absolute and line up with [`from_messages`] over the run's
/// committed history. Items are provisional until the run finishes.
pub struct RunTranscript {
    assistant_index: usize,
    block_count: usize,
    result_message: Option<usize>,
    result_block_count: usize,
    in_segment: bool,
    streamed_turns: usize,
    blocks: HashMap<usize, Option<String>>,
    segment_keys: Vec<Vec<String>>,
    calls: HashMap<String, String>,
    streaming_key: Option<String>,
    items: Vec<TranscriptItem>,
}

impl RunTranscript {
    /// Start folding a run that began after `base` stored messages.
    pub fn new(base: usize) -> Self {
        Self {
            assistant_index: base + 1,
            block_count: 0,
            result_message: None,
            result_block_count: 0,
            in_segment: false,
            streamed_turns: 0,
            blocks: HashMap::new(),
            segment_keys: Vec::new(),
            calls: HashMap::new(),
            streaming_key: None,
            items: Vec::new(),
        }
    }

    /// Fold one stream item into transcript events.
    pub fn apply(&mut self, item: MultiTurnStreamItem) -> Result<Vec<ChatEvent>> {
        match item {
            MultiTurnStreamItem::StreamAssistantItem(Item::Event(event)) => {
                Ok(self.apply_event(event))
            }
            MultiTurnStreamItem::StreamAssistantItem(Item::Unknown(_)) => Ok(Vec::new()),
            MultiTurnStreamItem::ToolCall { tool_call }
            | MultiTurnStreamItem::ToolExecutionCommitted { tool_call } => {
                Ok(self.tool_call_committed(&tool_call))
            }
            MultiTurnStreamItem::ToolResult { tool_result } => Ok(self.tool_result(&tool_result)),
            MultiTurnStreamItem::ModelTurnRetried { turn } => self.retry(turn),
            MultiTurnStreamItem::CompletionCall(_) | MultiTurnStreamItem::FinalResponse(_) => {
                Ok(Vec::new())
            }
            _ => Ok(Vec::new()),
        }
    }

    fn apply_event(&mut self, event: StreamEvent) -> Vec<ChatEvent> {
        match event {
            StreamEvent::Start { part, kind, name } => {
                self.open(part.index(), kind, name.map(|name| name.to_string()))
            }
            StreamEvent::Text { part, text } => self.append_text(part.index(), &text, true),
            StreamEvent::Reasoning { part, text } => self.append_text(part.index(), &text, false),
            StreamEvent::Arguments { part, json } => self.append_arguments(part.index(), &json),
            StreamEvent::End { part, content } => self.end(part.index(), content),
        }
    }

    fn open(&mut self, part: usize, kind: PartKind, name: Option<String>) -> Vec<ChatEvent> {
        let mut events = Vec::new();
        if let Some(event) = self.clear_streaming() {
            events.push(event);
        }

        let block_index = self.start_block();
        let (key, item) = match kind {
            PartKind::Text => {
                let key = self.new_key(block_index);
                let item = self.new_item(key.clone(), TranscriptKind::AssistantText, "", "");
                (Some(key), Some(item))
            }
            PartKind::Reasoning => {
                let key = self.new_key(block_index);
                let item = self.new_item(key.clone(), TranscriptKind::Reasoning, "", "");
                (Some(key), Some(item))
            }
            PartKind::ToolCall => {
                let key = self.new_key(block_index);
                let item = self.new_item(
                    key.clone(),
                    TranscriptKind::ToolCall,
                    name.as_deref().unwrap_or(""),
                    "",
                );
                (Some(key), Some(item))
            }
            PartKind::Image | PartKind::Opaque => (None, None),
        };

        self.blocks.insert(part, key.clone());
        if let Some(key) = &key {
            self.current_segment_keys().push(key.clone());
        }
        if let Some(item) = item {
            events.push(self.upsert(item));
        }
        events
    }

    fn append_text(&mut self, part: usize, text: &str, streaming: bool) -> Vec<ChatEvent> {
        let Some(Some(key)) = self.blocks.get(&part).cloned() else {
            return Vec::new();
        };
        let mut events = Vec::new();
        if streaming && self.streaming_key.as_deref() != Some(&key) {
            if let Some(event) = self.clear_streaming() {
                events.push(event);
            }
            self.streaming_key = Some(key.clone());
        }
        if let Some(item) = self.item_mut(&key) {
            item.text.push_str(text);
            item.streaming = streaming;
            events.push(ChatEvent::ItemUpserted { item: item.clone() });
        }
        events
    }

    fn append_arguments(&mut self, part: usize, json: &str) -> Vec<ChatEvent> {
        let Some(Some(key)) = self.blocks.get(&part).cloned() else {
            return Vec::new();
        };
        let Some(item) = self.item_mut(&key) else {
            return Vec::new();
        };
        item.text.push_str(json);
        vec![ChatEvent::ItemUpserted { item: item.clone() }]
    }

    fn end(&mut self, part: usize, content: AssistantContent) -> Vec<ChatEvent> {
        let Some(Some(key)) = self.blocks.get(&part).cloned() else {
            return Vec::new();
        };
        if self.streaming_key.as_deref() == Some(&key) {
            self.streaming_key = None;
        }
        let mut call_id = None;
        let updated = {
            let Some(item) = self.item_mut(&key) else {
                return Vec::new();
            };
            match content {
                AssistantContent::Text(text) => {
                    item.text = text.text;
                    item.streaming = false;
                }
                AssistantContent::Reasoning(reasoning) => {
                    item.text = reasoning.text;
                    item.streaming = false;
                }
                AssistantContent::ToolCall(call) => {
                    item.name = call.function.name.to_string();
                    item.text = tool_call_text(&call);
                    item.streaming = false;
                    call_id = Some(call.id.to_string());
                }
                _ => return Vec::new(),
            }
            item.clone()
        };
        if let Some(id) = call_id {
            self.calls.insert(id, key);
        }
        vec![ChatEvent::ItemUpserted { item: updated }]
    }

    fn tool_call_committed(&mut self, call: &ToolCall) -> Vec<ChatEvent> {
        let mut events = Vec::new();
        if let Some(event) = self.clear_streaming() {
            events.push(event);
        }
        let id = call.id.to_string();
        match self.calls.get(&id).cloned() {
            Some(key) => {
                if let Some(item) = self.item_mut(&key) {
                    item.name = call.function.name.to_string();
                    item.text = tool_call_text(call);
                    item.executing = true;
                    item.streaming = false;
                    events.push(ChatEvent::ItemUpserted { item: item.clone() });
                }
            }
            None => {
                let block_index = self.start_block();
                let key = self.new_key(block_index);
                self.current_segment_keys().push(key.clone());
                self.calls.insert(id, key.clone());
                let mut item = self.new_item(
                    key,
                    TranscriptKind::ToolCall,
                    &call.function.name.to_string(),
                    &tool_call_text(call),
                );
                item.executing = true;
                events.push(self.upsert(item));
            }
        }
        events
    }

    fn tool_result(&mut self, result: &ToolResult) -> Vec<ChatEvent> {
        let mut events = Vec::new();
        if let Some(event) = self.clear_streaming() {
            events.push(event);
        }

        let call_id = result.call.to_string();
        if let Some(key) = self.calls.get(&call_id).cloned() {
            if let Some(item) = self.item_mut(&key) {
                item.executing = false;
                events.push(ChatEvent::ItemUpserted { item: item.clone() });
            }
        }

        if self.result_message.is_none() {
            self.result_message = Some(self.assistant_index + 1);
            self.result_block_count = 0;
        }
        self.in_segment = false;
        let message_index = self
            .result_message
            .expect("tool result message index is set above");
        let block_index = self.result_block_count;
        self.result_block_count += 1;
        let item = TranscriptItem {
            key: block_key(message_index, block_index),
            kind: TranscriptKind::ToolResult,
            name: result.name.to_string(),
            text: tool_result_text(&result.content),
            provisional: true,
            is_error: result.is_error,
            executing: false,
            streaming: false,
        };
        events.push(self.upsert(item));
        events
    }

    fn retry(&mut self, turn: usize) -> Result<Vec<ChatEvent>> {
        let mut events = Vec::new();
        if let Some(event) = self.clear_streaming() {
            events.push(event);
        }

        if turn == self.streamed_turns {
            let removed = self.segment_keys.pop().ok_or_else(|| {
                anyhow!("model turn {turn} was retried without a streamed segment")
            })?;
            self.in_segment = false;
            self.blocks.clear();
            self.block_count = 0;
            self.calls.retain(|_, key| !removed.contains(key));
            for key in removed {
                if let Some(index) = self.items.iter().position(|item| item.key == key) {
                    self.items.remove(index);
                    events.push(ChatEvent::ItemRemoved { key });
                }
            }
        } else if turn > self.streamed_turns {
            // The turn streamed no output; there is no trailing segment to drop.
        } else {
            return Err(anyhow!(
                "model turn {turn} was retried after {} segments had already streamed",
                self.streamed_turns
            ));
        }
        Ok(events)
    }

    /// Remove the live streaming flag from the item that holds it.
    pub fn clear_streaming(&mut self) -> Option<ChatEvent> {
        let key = self.streaming_key.take()?;
        let item = self.item_mut(&key)?;
        item.streaming = false;
        Some(ChatEvent::ItemUpserted { item: item.clone() })
    }

    /// Start the assistant message a new block belongs to, returning the
    /// block's index within that message.
    fn start_block(&mut self) -> usize {
        if let Some(message_index) = self.result_message.take() {
            self.assistant_index = message_index + 1;
            self.block_count = 0;
            self.blocks.clear();
            self.in_segment = false;
        }
        if !self.in_segment {
            self.segment_keys.push(Vec::new());
            self.in_segment = true;
            self.streamed_turns += 1;
        }
        let block_index = self.block_count;
        self.block_count += 1;
        block_index
    }

    fn new_key(&self, block_index: usize) -> String {
        block_key(self.assistant_index, block_index)
    }

    fn current_segment_keys(&mut self) -> &mut Vec<String> {
        self.segment_keys
            .last_mut()
            .expect("a block starts after its segment is opened")
    }

    fn new_item(
        &self,
        key: String,
        kind: TranscriptKind,
        name: &str,
        text: &str,
    ) -> TranscriptItem {
        TranscriptItem {
            key,
            kind,
            name: name.to_string(),
            text: text.to_string(),
            provisional: true,
            is_error: false,
            executing: false,
            streaming: false,
        }
    }

    fn upsert(&mut self, item: TranscriptItem) -> ChatEvent {
        match self
            .items
            .iter()
            .position(|existing| existing.key == item.key)
        {
            Some(index) => self.items[index] = item.clone(),
            None => self.items.push(item.clone()),
        }
        ChatEvent::ItemUpserted { item }
    }

    fn item_mut(&mut self, key: &str) -> Option<&mut TranscriptItem> {
        self.items.iter_mut().find(|item| item.key == key)
    }
}

fn block_key(message_index: usize, block_index: usize) -> String {
    format!("{message_index}-{block_index}")
}

/// The text of a tool call: its invalid arguments verbatim when parsing kept
/// them, otherwise its arguments as pretty-printed JSON.
fn tool_call_text(call: &ToolCall) -> String {
    match &call.function.invalid_arguments {
        Some(raw) => raw.clone(),
        None => pretty_json(&serde_json::Value::Object(call.function.arguments.clone())),
    }
}

/// Flatten a tool result's content into one text body.
fn tool_result_text(content: &[ToolResultContent]) -> String {
    content
        .iter()
        .filter_map(|part| match part {
            ToolResultContent::Text(text) => Some(text.text.clone()),
            ToolResultContent::Json { value } => Some(pretty_json(value)),
            ToolResultContent::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn pretty_json(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).expect("a JSON value always serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::completion::AssistantMessage;
    use rig::message::{CallId, ToolFunction, ToolName};
    use rig::streaming::Part;

    #[test]
    fn stored_tool_call_and_result_normalize() {
        let call = ToolCall::new(
            CallId::from_wire("call-1"),
            ToolFunction::new(
                ToolName::new("search").unwrap(),
                serde_json::json!({ "query": "climate" }),
            ),
        );
        let result = ToolResult {
            call: call.id.clone(),
            name: call.function.name.clone(),
            content: vec![
                ToolResultContent::text("first line"),
                ToolResultContent::Json {
                    value: serde_json::json!({ "hits": 0 }),
                },
            ],
            is_error: true,
        };
        let messages = vec![
            Message::user("What do the reports say?"),
            Message::Assistant(AssistantMessage::new(vec![AssistantContent::ToolCall(
                call,
            )])),
            Message::User {
                content: vec![UserContent::ToolResult(result)],
            },
            Message::assistant("No matching passages."),
        ];

        let items = from_messages(&messages, 0);
        assert_eq!(items.len(), 4);

        assert_eq!(items[0].key, "0-0");
        assert_eq!(items[0].kind, TranscriptKind::UserText);
        assert_eq!(items[0].text, "What do the reports say?");

        assert_eq!(items[1].key, "1-0");
        assert_eq!(items[1].kind, TranscriptKind::ToolCall);
        assert_eq!(items[1].name, "search");
        assert_eq!(items[1].text, "{\n  \"query\": \"climate\"\n}");

        assert_eq!(items[2].key, "2-0");
        assert_eq!(items[2].kind, TranscriptKind::ToolResult);
        assert_eq!(items[2].name, "search");
        assert_eq!(items[2].text, "first line\n{\n  \"hits\": 0\n}");
        assert!(items[2].is_error);

        assert_eq!(items[3].key, "3-0");
        assert_eq!(items[3].kind, TranscriptKind::AssistantText);
        assert_eq!(items[3].text, "No matching passages.");

        assert!(items.iter().all(|item| !item.provisional));
        assert!(items.iter().all(|item| !item.executing && !item.streaming));
    }

    #[test]
    fn keys_are_absolute_from_the_base() {
        let messages = vec![Message::user("hello"), Message::assistant("hi")];
        let items = from_messages(&messages, 7);
        assert_eq!(items[0].key, "7-0");
        assert_eq!(items[1].key, "8-0");
    }

    fn stream_item(event: StreamEvent) -> MultiTurnStreamItem {
        MultiTurnStreamItem::StreamAssistantItem(Item::Event(event))
    }

    #[test]
    fn retry_drops_the_trailing_segment_and_reuses_its_keys() {
        let mut transcript = RunTranscript::new(0);
        let part = Part::new(0);

        let events = transcript
            .apply(stream_item(StreamEvent::Start {
                part,
                kind: PartKind::Text,
                name: None,
            }))
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ChatEvent::ItemUpserted { item }] if item.key == "1-0"
        ));

        let events = transcript
            .apply(stream_item(StreamEvent::Text {
                part,
                text: "provisional".into(),
            }))
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ChatEvent::ItemUpserted { item }] if item.streaming
        ));

        let events = transcript
            .apply(MultiTurnStreamItem::ModelTurnRetried { turn: 1 })
            .unwrap();
        assert!(matches!(
            events.first(),
            Some(ChatEvent::ItemUpserted { item }) if !item.streaming
        ));
        assert!(matches!(
            events.last(),
            Some(ChatEvent::ItemRemoved { key }) if key == "1-0"
        ));

        let events = transcript
            .apply(stream_item(StreamEvent::Start {
                part,
                kind: PartKind::Text,
                name: None,
            }))
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [ChatEvent::ItemUpserted { item }] if item.key == "1-0"
        ));
    }

    #[test]
    fn retry_below_the_streamed_segment_count_is_an_error() {
        let mut transcript = RunTranscript::new(0);
        transcript
            .apply(stream_item(StreamEvent::Start {
                part: Part::new(0),
                kind: PartKind::Text,
                name: None,
            }))
            .unwrap();

        let error = transcript
            .apply(MultiTurnStreamItem::ModelTurnRetried { turn: 0 })
            .unwrap_err();
        assert!(error.to_string().contains("turn 0"));
    }
}
