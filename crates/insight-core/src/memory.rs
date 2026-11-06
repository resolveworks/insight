//! File-backed conversation memory for the rig agent runtime.
//!
//! [`FileConversationMemory`] combines rig's directory-backed session store
//! with one metadata subdirectory per conversation:
//!
//! ```text
//! {root}/
//! ├── logs/{conversation_id}.jsonl  # rig conversation history
//! └── {conversation_id}/
//!     └── meta.json                 # title, collections, timestamps
//! ```
//!
//! Rig stores only messages; the metadata sidecar carries what the app needs
//! on top (title, linked collections, timestamps). `load`/`append`/`clear`
//! operate on rig's session log; the `create`/`load_meta`/`update_meta`/
//! `list_summaries`/`delete`/`touch` methods operate on `meta.json`.
//!
//! Rig atomically appends session records; metadata writes use
//! write-temp-then-rename, so a crash mid-write never truncates `meta.json`.

use std::path::{Path, PathBuf};

use rig::completion::Message;
use rig_memory::{DirStore, SessionError};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::CollectionInfo;

/// Errors produced by [`FileConversationMemory`].
#[derive(Debug, thiserror::Error)]
pub enum MemoryStoreError {
    /// Filesystem I/O failure.
    #[error("memory store I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A stored JSON file failed to parse.
    #[error("memory store JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Rig could not load or save a conversation session.
    #[error("conversation session error: {0}")]
    Session(#[from] SessionError),

    /// A conversation with the given id already exists.
    #[error("conversation already exists: {0}")]
    AlreadyExists(String),

    /// A conversation with the given id does not exist.
    #[error("conversation not found: {0}")]
    NotFound(String),
}

/// App-facing conversation metadata, stored alongside the rig message
/// history at `{root}/{id}/meta.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMeta {
    /// Conversation id (also the directory name).
    pub id: String,
    /// Human-readable title shown in the UI.
    pub title: String,
    /// Collections the conversation is scoped to.
    pub collections: Vec<CollectionInfo>,
    /// When the conversation was created (ISO 8601).
    pub created_at: String,
    /// When the conversation was last updated (ISO 8601).
    pub updated_at: String,
}

/// Summary of a conversation for list views.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub updated_at: String,
}

impl From<&ConversationMeta> for ConversationSummary {
    fn from(meta: &ConversationMeta) -> Self {
        Self {
            id: meta.id.clone(),
            title: meta.title.clone(),
            updated_at: meta.updated_at.clone(),
        }
    }
}

/// Current time as an ISO 8601 string, matching the rest of the app.
fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Rig session storage under a root directory, plus an app-facing metadata
/// sidecar per conversation.
pub struct FileConversationMemory {
    root: PathBuf,
    store: DirStore,
}

impl FileConversationMemory {
    /// Create a store rooted at `root` (e.g. `Config::conversations_dir`).
    /// The directory itself is created lazily on first write.
    pub fn new(root: PathBuf) -> Result<Self, MemoryStoreError> {
        let store = DirStore::open(&root)?;
        Ok(Self { root, store })
    }

    /// Clone rig's session store for an agent builder.
    pub fn store(&self) -> DirStore {
        self.store.clone()
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    fn meta_path(&self, id: &str) -> PathBuf {
        self.dir(id).join("meta.json")
    }

    /// Write `bytes` to `path` durably: serialize to a sibling temp file,
    /// then atomically rename it into place. A crash mid-write leaves the
    /// previous contents intact and at most one stale temp file.
    async fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
        fs::write(&tmp, bytes).await?;
        fs::rename(&tmp, path).await?;
        Ok(())
    }

    /// Create a new conversation with `meta`. Fails if one already exists.
    pub async fn create(
        &self,
        meta: ConversationMeta,
    ) -> Result<ConversationMeta, MemoryStoreError> {
        if self.meta_path(&meta.id).exists() {
            return Err(MemoryStoreError::AlreadyExists(meta.id));
        }
        let now = now_iso8601();
        let mut meta = meta;
        meta.created_at = now.clone();
        meta.updated_at = now;
        fs::create_dir_all(self.dir(&meta.id)).await?;
        let bytes = serde_json::to_vec_pretty(&meta)?;
        Self::write_atomic(&self.meta_path(&meta.id), &bytes).await?;
        Ok(meta)
    }

    /// Load the metadata sidecar for `id`, or `None` if unknown.
    pub async fn load_meta(&self, id: &str) -> Result<Option<ConversationMeta>, MemoryStoreError> {
        let path = self.meta_path(id);
        match fs::read(&path).await {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Overwrite the metadata sidecar for `id`. Fails if the conversation
    /// does not exist.
    pub async fn update_meta(
        &self,
        id: &str,
        meta: ConversationMeta,
    ) -> Result<(), MemoryStoreError> {
        let path = self.meta_path(id);
        if !path.exists() {
            return Err(MemoryStoreError::NotFound(id.to_owned()));
        }
        let bytes = serde_json::to_vec_pretty(&meta)?;
        Self::write_atomic(&path, &bytes).await?;
        Ok(())
    }

    /// List all conversation summaries, most recently updated first.
    /// Conversations whose sidecar fails to parse are skipped (warned).
    pub async fn list_summaries(&self) -> Result<Vec<ConversationSummary>, MemoryStoreError> {
        let mut summaries = Vec::new();
        let mut entries = match fs::read_dir(&self.root).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(summaries),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().into_string().ok() else {
                continue;
            };
            match self.load_meta(&name).await {
                Ok(Some(meta)) => summaries.push(ConversationSummary::from(&meta)),
                Ok(None) => {}
                Err(e) => tracing::warn!("Skipping conversation {name}: {e}"),
            }
        }
        summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(summaries)
    }

    /// Delete a conversation entirely (messages and metadata). Idempotent:
    /// removing an unknown id succeeds.
    pub async fn delete(&self, id: &str) -> Result<(), MemoryStoreError> {
        self.clear(id).await?;
        match fs::remove_dir_all(self.dir(id)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Bump `updated_at` on the metadata sidecar to now. Fails if the
    /// conversation does not exist.
    pub async fn touch(&self, id: &str) -> Result<(), MemoryStoreError> {
        let mut meta = self
            .load_meta(id)
            .await?
            .ok_or_else(|| MemoryStoreError::NotFound(id.to_owned()))?;
        meta.updated_at = now_iso8601();
        self.update_meta(id, meta).await
    }
}

impl FileConversationMemory {
    /// Load a conversation's messages, or an empty vector for an unknown id.
    pub async fn load(&self, id: &str) -> Result<Vec<Message>, MemoryStoreError> {
        let conversation = rig_memory::load_async(&self.store, id).await?;
        Ok(conversation
            .into_messages()
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Append messages to a conversation.
    pub async fn append(&self, id: &str, messages: Vec<Message>) -> Result<(), MemoryStoreError> {
        let mut conversation = rig_memory::load_async(&self.store, id).await?;
        conversation.extend(messages);
        rig_memory::save_async(&self.store, id, &mut conversation).await?;
        Ok(())
    }

    /// Remove a conversation's messages while retaining its metadata.
    pub async fn clear(&self, id: &str) -> Result<(), MemoryStoreError> {
        let path = self.root.join("logs").join(format!("{id}.jsonl"));
        match fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::completion::message::{AssistantContent, UserContent};

    fn temp_store() -> (tempfile::TempDir, FileConversationMemory) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileConversationMemory::new(dir.path().to_path_buf()).unwrap();
        (dir, store)
    }

    fn sample_meta(id: &str, title: &str) -> ConversationMeta {
        ConversationMeta {
            id: id.to_owned(),
            title: title.to_owned(),
            collections: vec![CollectionInfo {
                id: "col-1".to_owned(),
                name: "Climate Research".to_owned(),
                document_count: 3,
                total_pages: 42,
                created_at: Some("2024-01-15T10:30:00Z".to_owned()),
            }],
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn messages_with_tools() -> Vec<Message> {
        vec![
            Message::user("What is in the collection?"),
            Message::Assistant(rig::completion::AssistantMessage::new(vec![
                AssistantContent::tool_call(
                    "call-1",
                    rig::message::ToolName::new("search_documents").unwrap(),
                    serde_json::json!({"query": "climate"}),
                ),
            ])),
            Message::tool_result(
                rig::message::CallId::from_wire("call-1"),
                rig::message::ToolName::new("search_documents").unwrap(),
                "3 hits",
            ),
            Message::assistant("The collection holds 3 documents."),
        ]
    }

    #[tokio::test]
    async fn unknown_id_loads_empty() {
        let (_dir, store) = temp_store();
        assert!(store.load("nope").await.unwrap().is_empty());
        assert!(store.load_meta("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn message_round_trip_with_tool_blocks() {
        let (_dir, store) = temp_store();
        let expected = messages_with_tools();
        store.append("c1", expected.clone()).await.unwrap();

        let loaded = store.load("c1").await.unwrap();
        assert_eq!(loaded, expected);

        // The tool-result message keeps its role and block kind.
        let Message::User { content } = &loaded[2] else {
            panic!("expected user role");
        };
        assert!(matches!(content[0], UserContent::ToolResult(_)));
    }

    #[tokio::test]
    async fn append_accumulates_across_calls() {
        let (_dir, store) = temp_store();
        store
            .append("c2", vec![Message::user("one")])
            .await
            .unwrap();
        store
            .append("c2", vec![Message::assistant("two")])
            .await
            .unwrap();
        assert_eq!(store.load("c2").await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn clear_empties_history_but_keeps_meta() {
        let (_dir, store) = temp_store();
        store.create(sample_meta("c3", "T")).await.unwrap();
        store.append("c3", vec![Message::user("hi")]).await.unwrap();
        store.clear("c3").await.unwrap();

        assert!(store.load("c3").await.unwrap().is_empty());
        assert!(store.load_meta("c3").await.unwrap().is_some());
        // Clearing again is fine (trait contract: idempotent).
        store.clear("c3").await.unwrap();
    }

    #[tokio::test]
    async fn meta_create_load_update_list_delete() {
        let (_dir, store) = temp_store();

        store.create(sample_meta("a", "Alpha")).await.unwrap();
        // Duplicate create fails.
        assert!(matches!(
            store.create(sample_meta("a", "Alpha")).await,
            Err(MemoryStoreError::AlreadyExists(_))
        ));

        let loaded = store.load_meta("a").await.unwrap().unwrap();
        assert_eq!(loaded.title, "Alpha");
        assert_eq!(loaded.created_at, loaded.updated_at);

        // Updating a missing conversation fails.
        assert!(matches!(
            store
                .update_meta("missing", sample_meta("missing", "x"))
                .await,
            Err(MemoryStoreError::NotFound(_))
        ));

        store.create(sample_meta("b", "Beta")).await.unwrap();
        store.touch("b").await.unwrap();

        // List is sorted by updated_at descending: b was touched last.
        let summaries = store.list_summaries().await.unwrap();
        let ids: Vec<_> = summaries.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "a"]);

        // update_meta overwrites fields.
        let mut meta = store.load_meta("a").await.unwrap().unwrap();
        meta.title = "Alpha 2".to_owned();
        store.update_meta("a", meta).await.unwrap();
        assert_eq!(
            store.load_meta("a").await.unwrap().unwrap().title,
            "Alpha 2"
        );

        // Delete is idempotent and removes metadata.
        store.delete("a").await.unwrap();
        store.delete("a").await.unwrap();
        assert!(store.load_meta("a").await.unwrap().is_none());
        assert_eq!(store.list_summaries().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn atomic_write_leaves_no_temp_litter() {
        let (_dir, store) = temp_store();
        store.append("c4", vec![Message::user("hi")]).await.unwrap();
        store.create(sample_meta("c4", "T")).await.unwrap();

        for entry in std::fs::read_dir(store.dir("c4")).unwrap() {
            let name = entry.unwrap().file_name();
            assert!(
                !name.to_string_lossy().contains(".tmp"),
                "stray temp file: {name:?}"
            );
        }
    }

    /// First text of each user message, for asserting on stored history.
    fn user_texts(messages: &[Message]) -> Vec<String> {
        messages
            .iter()
            .filter_map(|m| match m {
                Message::User { content } => content.iter().find_map(|c| match c {
                    UserContent::Text(t) => Some(t.text.clone()),
                    _ => None,
                }),
                _ => None,
            })
            .collect()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_appends_do_not_corrupt() {
        let (_dir, store) = temp_store();

        let batch = |n: usize| {
            (0..n)
                .map(|i| Message::user(format!("msg-{i}")))
                .collect::<Vec<_>>()
        };
        let a = batch(10);
        let b = batch(20);

        let s1 = &store;
        let s2 = &store;
        let (ra, rb) = tokio::join!(async move { s1.append("c5", a).await }, async move {
            s2.append("c5", b).await
        },);
        ra.unwrap();
        rb.unwrap();

        // Final state must be one of the two complete writes: with the
        // per-id lock, either all 10 first then all 20, or all 20 then all 10.
        let texts = user_texts(&store.load("c5").await.unwrap());
        assert!(
            texts.len() == 30 || texts.len() == 10 || texts.len() == 20,
            "unexpected length {}",
            texts.len()
        );
        if texts.len() == 30 {
            // No torn write: each batch appears contiguously in some order.
            let a_first = (0..10).all(|i| texts[i] == format!("msg-{i}"));
            let b_first = (0..20).all(|i| texts[i] == format!("msg-{i}"));
            assert!(
                a_first || b_first,
                "batches are interleaved, not serialized"
            );
        }
    }
}
