//! File-backed conversation memory for the rig agent runtime.
//!
//! [`FileConversationMemory`] combines rig's directory-backed session store
//! with app-owned metadata in a dedicated namespace:
//!
//! ```text
//! {root}/
//! ├── logs/{conversation_id}.jsonl  # rig conversation history
//! ├── metadata/{conversation_id}.json # title, collections, timestamps
//! └── files/...                     # rig blobs
//! ```
//!
//! Rig stores only messages; the metadata sidecar carries what the app needs
//! on top (title, linked collections, timestamps). `load` reads rig's session
//! log; `create`/`load_meta`/`update_meta`/`list_summaries`/`delete`/`touch`
//! operate on metadata.
//!
//! Metadata changes are read-modify-write transactions serialized by one
//! process-wide lock, so concurrent updates of different fields cannot lose
//! each other. Both stores write atomically: a crash mid-write leaves the
//! previous file intact.

use std::path::PathBuf;

use rig::completion::Message;
use rig_memory::{DirFiles, DirStore, Files, SessionError};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

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

    /// A conversation still uses the pre-rig-session on-disk layout.
    #[error(
        "conversation {0} uses the legacy conversations/<id>/messages.json layout; \
         remove it or convert it before using this version"
    )]
    LegacyLayout(String),

    /// The metadata namespace holds a file that is not a conversation meta.
    #[error("invalid conversation metadata file: {0}")]
    InvalidMetadata(String),
}

/// App-facing conversation metadata, stored at `{root}/metadata/{id}.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMeta {
    /// Conversation id (also the metadata file name).
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

/// The metadata file name for a conversation id.
fn meta_name(id: &str) -> String {
    format!("{id}.json")
}

/// Rig session storage under a root directory, plus app-facing metadata.
pub struct FileConversationMemory {
    root: PathBuf,
    store: DirStore,
    metadata: DirFiles,
    /// Serializes every metadata read-modify-write transaction.
    metadata_lock: Mutex<()>,
}

impl FileConversationMemory {
    /// Create a store rooted at `root` (e.g. `Config::conversations_dir`).
    /// The directories themselves are created lazily on first write.
    pub fn new(root: PathBuf) -> Result<Self, MemoryStoreError> {
        let store = DirStore::open(&root)?;
        let metadata = DirFiles::new(root.join("metadata"));
        Ok(Self {
            root,
            store,
            metadata,
            metadata_lock: Mutex::new(()),
        })
    }

    /// Clone rig's session store for an agent builder.
    pub fn store(&self) -> DirStore {
        self.store.clone()
    }

    /// A conversation stored in the legacy pre-rig-session layout.
    fn ensure_not_legacy(&self, id: &str) -> Result<(), MemoryStoreError> {
        let legacy = self.root.join(id).join("messages.json");
        if legacy.is_file() {
            return Err(MemoryStoreError::LegacyLayout(id.to_owned()));
        }
        Ok(())
    }

    /// Refuse a root directory holding any legacy conversation, so listing
    /// surfaces the old layout instead of silently omitting it.
    fn ensure_no_legacy_conversations(&self) -> Result<(), MemoryStoreError> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            self.ensure_not_legacy(name)?;
        }
        Ok(())
    }

    fn read_metadata(&self, id: &str) -> Result<Option<ConversationMeta>, MemoryStoreError> {
        match self.metadata.read_file(&meta_name(id)) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn write_metadata(&self, meta: &ConversationMeta) -> Result<(), MemoryStoreError> {
        let bytes = serde_json::to_vec_pretty(meta)?;
        self.metadata.write_file(&meta_name(&meta.id), &bytes)?;
        Ok(())
    }

    /// Create a new conversation with `meta`. Fails if one already exists.
    pub async fn create(
        &self,
        meta: ConversationMeta,
    ) -> Result<ConversationMeta, MemoryStoreError> {
        let _guard = self.metadata_lock.lock().await;
        self.ensure_not_legacy(&meta.id)?;
        if self.read_metadata(&meta.id)?.is_some() {
            return Err(MemoryStoreError::AlreadyExists(meta.id));
        }
        let now = now_iso8601();
        let mut meta = meta;
        meta.created_at = now.clone();
        meta.updated_at = now;
        self.write_metadata(&meta)?;
        Ok(meta)
    }

    /// Load the metadata sidecar for `id`, or `None` if unknown.
    pub async fn load_meta(&self, id: &str) -> Result<Option<ConversationMeta>, MemoryStoreError> {
        self.ensure_not_legacy(id)?;
        self.read_metadata(id)
    }

    /// Apply `update` to the metadata sidecar for `id` in one serialized
    /// transaction. Fails if the conversation does not exist.
    pub async fn update_meta(
        &self,
        id: &str,
        update: impl FnOnce(&mut ConversationMeta),
    ) -> Result<(), MemoryStoreError> {
        let _guard = self.metadata_lock.lock().await;
        self.ensure_not_legacy(id)?;
        let mut meta = self
            .read_metadata(id)?
            .ok_or_else(|| MemoryStoreError::NotFound(id.to_owned()))?;
        update(&mut meta);
        self.write_metadata(&meta)
    }

    /// List all conversation summaries, most recently updated first.
    /// Malformed metadata is an error, never silently skipped.
    pub async fn list_summaries(&self) -> Result<Vec<ConversationSummary>, MemoryStoreError> {
        self.ensure_no_legacy_conversations()?;
        let mut summaries = Vec::new();
        for name in self.metadata.files()? {
            let Some(id) = name.strip_suffix(".json") else {
                return Err(MemoryStoreError::InvalidMetadata(name));
            };
            let bytes = match self.metadata.read_file(&name) {
                Ok(bytes) => bytes,
                // Another process removed it between listing and reading.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            let meta: ConversationMeta = serde_json::from_slice(&bytes)?;
            if id != meta.id {
                return Err(MemoryStoreError::InvalidMetadata(name));
            }
            summaries.push(ConversationSummary::from(&meta));
        }
        summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(summaries)
    }

    /// Delete a conversation entirely (messages and metadata). Idempotent:
    /// removing an unknown id succeeds.
    pub async fn delete(&self, id: &str) -> Result<(), MemoryStoreError> {
        let _guard = self.metadata_lock.lock().await;
        self.remove_session_log(id).await?;
        self.metadata.remove_file(&meta_name(id))?;
        Ok(())
    }

    /// Bump `updated_at` on the metadata sidecar to now. Fails if the
    /// conversation does not exist.
    pub async fn touch(&self, id: &str) -> Result<(), MemoryStoreError> {
        self.update_meta(id, |meta| meta.updated_at = now_iso8601())
            .await
    }

    /// rig-memory exposes no per-log removal, so this is the only place that
    /// knows its `logs/{id}.jsonl` layout.
    async fn remove_session_log(&self, id: &str) -> Result<(), MemoryStoreError> {
        let path = self.root.join("logs").join(format!("{id}.jsonl"));
        match tokio::fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

impl FileConversationMemory {
    /// Load a conversation's messages, or an empty vector for a conversation
    /// that has not recorded a message yet.
    pub async fn load(&self, id: &str) -> Result<Vec<Message>, MemoryStoreError> {
        self.ensure_not_legacy(id)?;
        let conversation = rig_memory::load_async(&self.store, id).await?;
        Ok(conversation
            .into_messages()
            .into_iter()
            .map(Into::into)
            .collect())
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

    fn collection(id: &str) -> CollectionInfo {
        CollectionInfo {
            id: id.to_owned(),
            name: format!("Collection {id}"),
            document_count: 1,
            total_pages: 2,
            created_at: None,
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

    /// Persist messages the way the agent runtime does.
    async fn save_messages(store: &FileConversationMemory, id: &str, messages: Vec<Message>) {
        let mut conversation = rig_memory::load_async(&store.store(), id).await.unwrap();
        conversation.extend(messages);
        rig_memory::save_async(&store.store(), id, &mut conversation)
            .await
            .unwrap();
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
        save_messages(&store, "c1", expected.clone()).await;

        let loaded = store.load("c1").await.unwrap();
        assert_eq!(loaded, expected);

        // The tool-result message keeps its role and block kind.
        let Message::User { content } = &loaded[2] else {
            panic!("expected user role");
        };
        assert!(matches!(content[0], UserContent::ToolResult(_)));
    }

    #[tokio::test]
    async fn delete_removes_history_and_meta() {
        let (_dir, store) = temp_store();
        store.create(sample_meta("c3", "T")).await.unwrap();
        save_messages(&store, "c3", vec![Message::user("hi")]).await;
        store.delete("c3").await.unwrap();

        assert!(store.load("c3").await.unwrap().is_empty());
        assert!(store.load_meta("c3").await.unwrap().is_none());
        // Deleting again is fine.
        store.delete("c3").await.unwrap();
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
                .update_meta("missing", |meta| meta.title = "x".into())
                .await,
            Err(MemoryStoreError::NotFound(_))
        ));

        store.create(sample_meta("b", "Beta")).await.unwrap();
        store.touch("b").await.unwrap();

        // List is sorted by updated_at descending: b was touched last.
        let summaries = store.list_summaries().await.unwrap();
        let ids: Vec<_> = summaries.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "a"]);

        // update_meta mutates only the fields it is given.
        store
            .update_meta("a", |meta| meta.title = "Alpha 2".to_owned())
            .await
            .unwrap();
        let meta = store.load_meta("a").await.unwrap().unwrap();
        assert_eq!(meta.title, "Alpha 2");
        assert_eq!(meta.collections.len(), loaded.collections.len());
        assert_eq!(meta.collections[0].id, loaded.collections[0].id);
        assert_eq!(meta.updated_at, loaded.updated_at);

        // Delete is idempotent and removes metadata.
        store.delete("a").await.unwrap();
        store.delete("a").await.unwrap();
        assert!(store.load_meta("a").await.unwrap().is_none());
        assert_eq!(store.list_summaries().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn metadata_writes_leave_no_temp_litter() {
        let (_dir, store) = temp_store();
        store.create(sample_meta("c4", "T")).await.unwrap();

        for entry in std::fs::read_dir(store.metadata.path()).unwrap() {
            let name = entry.unwrap().file_name();
            assert!(
                !name.to_string_lossy().contains("rig-part"),
                "stray part file: {name:?}"
            );
        }
    }

    #[tokio::test]
    async fn corrupt_metadata_fails_listing() {
        let (_dir, store) = temp_store();
        store.create(sample_meta("good", "Good")).await.unwrap();
        std::fs::write(store.metadata.path().join("bad.json"), b"{not json").unwrap();

        assert!(matches!(
            store.list_summaries().await,
            Err(MemoryStoreError::Json(_))
        ));
    }

    #[tokio::test]
    async fn legacy_layout_is_refused() {
        let (dir, store) = temp_store();
        let legacy = dir.path().join("old");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("messages.json"), b"[]").unwrap();

        assert!(matches!(
            store.list_summaries().await,
            Err(MemoryStoreError::LegacyLayout(id)) if id == "old"
        ));
        assert!(matches!(
            store.load_meta("old").await,
            Err(MemoryStoreError::LegacyLayout(_))
        ));
        assert!(matches!(
            store.load("old").await,
            Err(MemoryStoreError::LegacyLayout(_))
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_touch_and_collection_update_keep_both() {
        let (_dir, store) = temp_store();
        store.create(sample_meta("c5", "T")).await.unwrap();

        for _ in 0..20 {
            let (touched, updated) = tokio::join!(async { store.touch("c5").await }, async {
                store
                    .update_meta("c5", |meta| {
                        meta.collections = vec![collection("col-2")];
                    })
                    .await
            },);
            touched.unwrap();
            updated.unwrap();

            let meta = store.load_meta("c5").await.unwrap().unwrap();
            assert_eq!(meta.collections.len(), 1);
            assert_eq!(meta.collections[0].id, "col-2");
        }
    }
}
