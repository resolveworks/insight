//! File-backed conversation memory for the rig agent runtime.
//!
//! A conversation belongs to exactly one evidence collection, fixed at
//! creation. [`FileConversationMemory`] keeps each conversation in its own
//! rig directory store under the collection:
//!
//! ```text
//! {root}/
//! └── {collection_id}/
//!     └── {conversation_id}/
//!         ├── logs/{conversation_id}.jsonl  # rig conversation history
//!         ├── files/meta.json               # rig SessionMeta
//!         └── .rig-memory.lock
//! ```
//!
//! Collection ids are canonical [`iroh_docs::NamespaceId`] strings, so the
//! directory names cannot escape the root. `SessionMeta` is the only
//! app-owned metadata: `title` and `updated` (milliseconds since the epoch).
//! An empty title means no title has been generated yet; the display
//! fallback ([`DEFAULT_TITLE`]) is applied when building summaries, never
//! persisted.

use std::path::PathBuf;
use std::str::FromStr;

use iroh_docs::NamespaceId;
use rig::completion::Message;
use rig_core::now_ms;
use rig_memory::{DirStore, Files, SessionError, SessionMeta, Store, META};
use serde::Serialize;

/// The title a conversation is shown with until one is generated.
pub const DEFAULT_TITLE: &str = "New conversation";

/// Errors produced by [`FileConversationMemory`].
#[derive(Debug, thiserror::Error)]
pub enum MemoryStoreError {
    /// Filesystem I/O failure.
    #[error("memory store I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Rig could not load or save a conversation session.
    #[error("conversation session error: {0}")]
    Session(#[from] SessionError),

    /// A conversation with the given id already exists.
    #[error("conversation already exists: {0}")]
    AlreadyExists(String),

    /// A conversation with the given id does not exist.
    #[error("conversation not found: {0}")]
    NotFound(String),

    /// A conversation directory has no readable metadata.
    #[error("invalid conversation metadata: {0}")]
    InvalidMetadata(String),

    /// A collection directory holds an entry whose name is not UTF-8.
    #[error("invalid conversations directory entry name: {0:?}")]
    InvalidEntry(std::ffi::OsString),

    /// The collection id is not a valid namespace id.
    #[error("invalid collection id: {0}")]
    InvalidCollection(String),

    /// The conversation id is not valid as a single path part.
    #[error("invalid conversation id: {0}")]
    InvalidConversationId(String),
}

/// Summary of a conversation for list views.
#[derive(Debug, Clone, Serialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
}

/// A conversation log kept in the rig directory store of one collection.
pub struct FileConversationMemory {
    root: PathBuf,
}

impl FileConversationMemory {
    /// Create a store rooted at `root` (e.g. `Config::conversations_dir`).
    /// The directories themselves are created lazily on first write.
    pub fn new(root: PathBuf) -> Result<Self, MemoryStoreError> {
        Ok(Self { root })
    }

    /// The canonical directory of `collection_id`.
    fn collection_dir(&self, collection_id: &str) -> Result<PathBuf, MemoryStoreError> {
        let namespace_id = NamespaceId::from_str(collection_id)
            .map_err(|_| MemoryStoreError::InvalidCollection(collection_id.to_owned()))?;
        Ok(self.root.join(namespace_id.to_string()))
    }

    /// The conversation directory, validating both path parts.
    fn conversation_dir(
        &self,
        collection_id: &str,
        conversation_id: &str,
    ) -> Result<PathBuf, MemoryStoreError> {
        let collection_dir = self.collection_dir(collection_id)?;
        rig_memory::check_log_name(conversation_id)
            .map_err(|_| MemoryStoreError::InvalidConversationId(conversation_id.to_owned()))?;
        Ok(collection_dir.join(conversation_id))
    }

    /// Open the rig directory store of one conversation.
    fn open(
        &self,
        collection_id: &str,
        conversation_id: &str,
    ) -> Result<(PathBuf, DirStore), MemoryStoreError> {
        let dir = self.conversation_dir(collection_id, conversation_id)?;
        let store = DirStore::open(&dir)?;
        Ok((dir, store))
    }

    fn read_meta(&self, store: &DirStore) -> Result<Option<SessionMeta>, MemoryStoreError> {
        if let Some(meta) = SessionMeta::read(store) {
            return Ok(Some(meta));
        }
        // `SessionMeta::read` reports a missing file and an unreadable one
        // alike; a file that is there must not read as no metadata.
        match store.read_file(META) {
            Ok(_) => Err(MemoryStoreError::InvalidMetadata(format!(
                "{META} does not parse"
            ))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn write_meta(&self, store: &DirStore, meta: &SessionMeta) -> Result<(), MemoryStoreError> {
        meta.write(store)?;
        Ok(())
    }

    /// Create a new conversation with an empty title. Fails if one already
    /// exists in the collection.
    pub async fn create(
        &self,
        collection_id: &str,
        conversation_id: &str,
    ) -> Result<SessionMeta, MemoryStoreError> {
        let (_, store) = self.open(collection_id, conversation_id)?;
        if self.read_meta(&store)?.is_some() {
            return Err(MemoryStoreError::AlreadyExists(format!(
                "{collection_id}/{conversation_id}"
            )));
        }
        let meta = SessionMeta {
            updated: now_ms(),
            ..Default::default()
        };
        self.write_meta(&store, &meta)?;
        Ok(meta)
    }

    /// Load the metadata of a conversation, or `None` if unknown.
    pub async fn load_meta(
        &self,
        collection_id: &str,
        conversation_id: &str,
    ) -> Result<Option<SessionMeta>, MemoryStoreError> {
        let (_, store) = self.open(collection_id, conversation_id)?;
        self.read_meta(&store)
    }

    /// Replace the title without changing `updated`. Fails if the
    /// conversation does not exist.
    pub async fn set_title(
        &self,
        collection_id: &str,
        conversation_id: &str,
        title: &str,
    ) -> Result<(), MemoryStoreError> {
        let (_, store) = self.open(collection_id, conversation_id)?;
        let mut meta = self.read_meta(&store)?.ok_or_else(|| {
            MemoryStoreError::NotFound(format!("{collection_id}/{conversation_id}"))
        })?;
        meta.title = title.to_owned();
        self.write_meta(&store, &meta)
    }

    /// Set `updated` to now. Fails if the conversation does not exist.
    pub async fn touch(
        &self,
        collection_id: &str,
        conversation_id: &str,
    ) -> Result<(), MemoryStoreError> {
        let (_, store) = self.open(collection_id, conversation_id)?;
        let mut meta = self.read_meta(&store)?.ok_or_else(|| {
            MemoryStoreError::NotFound(format!("{collection_id}/{conversation_id}"))
        })?;
        meta.updated = now_ms();
        self.write_meta(&store, &meta)
    }

    /// List the conversations of a collection, most recently updated first.
    /// A missing collection directory lists empty.
    pub async fn list_summaries(
        &self,
        collection_id: &str,
    ) -> Result<Vec<ConversationSummary>, MemoryStoreError> {
        let collection_dir = self.collection_dir(collection_id)?;
        let entries = match std::fs::read_dir(&collection_dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        let mut summaries = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let Some(conversation_id) = name.to_str() else {
                return Err(MemoryStoreError::InvalidEntry(name));
            };
            if !entry.file_type()?.is_dir() {
                continue;
            }

            let (_, store) = self.open(collection_id, conversation_id)?;
            let meta = self.read_meta(&store)?.ok_or_else(|| {
                MemoryStoreError::InvalidMetadata(format!("{collection_id}/{conversation_id}"))
            })?;
            summaries.push((
                meta.updated,
                ConversationSummary {
                    id: conversation_id.to_owned(),
                    title: if meta.title.is_empty() {
                        DEFAULT_TITLE.to_owned()
                    } else {
                        meta.title.clone()
                    },
                },
            ));
        }

        summaries.sort_by_key(|a| std::cmp::Reverse(a.0));
        Ok(summaries.into_iter().map(|(_, summary)| summary).collect())
    }

    /// Delete a conversation and its history. Idempotent: removing an
    /// unknown conversation succeeds.
    pub async fn delete(
        &self,
        collection_id: &str,
        conversation_id: &str,
    ) -> Result<(), MemoryStoreError> {
        let (dir, store) = self.open(collection_id, conversation_id)?;
        store.destroy()?;
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete a collection's entire conversation directory. Idempotent.
    pub async fn delete_collection(&self, collection_id: &str) -> Result<(), MemoryStoreError> {
        let dir = self.collection_dir(collection_id)?;
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Load a conversation's messages, or an empty vector for a conversation
    /// that has not recorded a message yet.
    pub async fn load(
        &self,
        collection_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<Message>, MemoryStoreError> {
        let (_, store) = self.open(collection_id, conversation_id)?;
        let conversation = rig_memory::load_async(&store, conversation_id).await?;
        Ok(conversation
            .into_messages()
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Clone the rig directory store of one conversation for an agent
    /// builder.
    pub fn store(
        &self,
        collection_id: &str,
        conversation_id: &str,
    ) -> Result<DirStore, MemoryStoreError> {
        let (_, store) = self.open(collection_id, conversation_id)?;
        Ok(store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::completion::message::AssistantContent;

    fn temp_store() -> (tempfile::TempDir, FileConversationMemory) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileConversationMemory::new(dir.path().to_path_buf()).unwrap();
        (dir, store)
    }

    fn collection_id(byte: u8) -> String {
        iroh_docs::NamespaceSecret::from_bytes(&[byte; 32])
            .id()
            .to_string()
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
    async fn save_messages(
        store: &FileConversationMemory,
        collection: &str,
        conversation: &str,
        messages: Vec<Message>,
    ) {
        let dir_store = store.store(collection, conversation).unwrap();
        let mut log = rig_memory::load_async(&dir_store, conversation)
            .await
            .unwrap();
        log.extend(messages);
        rig_memory::save_async(&dir_store, conversation, &mut log)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn create_and_load_meta_round_trip() {
        let (_dir, store) = temp_store();
        let collection = collection_id(1);

        let created = store.create(&collection, "conv").await.unwrap();
        assert!(created.title.is_empty());
        assert!(created.updated > 0);

        let loaded = store.load_meta(&collection, "conv").await.unwrap().unwrap();
        assert_eq!(loaded, created);
    }

    #[tokio::test]
    async fn same_id_in_two_collections_is_independent() {
        let (_dir, store) = temp_store();
        let first = collection_id(1);
        let second = collection_id(2);

        store.create(&first, "conv").await.unwrap();
        store.create(&second, "conv").await.unwrap();
        store.set_title(&first, "conv", "First").await.unwrap();

        assert_eq!(
            store
                .load_meta(&first, "conv")
                .await
                .unwrap()
                .unwrap()
                .title,
            "First"
        );
        assert!(store
            .load_meta(&second, "conv")
            .await
            .unwrap()
            .unwrap()
            .title
            .is_empty());
        assert!(
            matches!(
                store.create(&first, "conv").await,
                Err(MemoryStoreError::AlreadyExists(_))
            ),
            "creating the same conversation twice fails"
        );
    }

    #[tokio::test]
    async fn wrong_collection_is_not_found() {
        let (_dir, store) = temp_store();
        let first = collection_id(1);
        let second = collection_id(2);

        store.create(&first, "conv").await.unwrap();
        save_messages(&store, &first, "conv", vec![Message::user("hi")]).await;

        assert!(store.load_meta(&second, "conv").await.unwrap().is_none());
        assert!(store.load(&second, "conv").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn set_title_preserves_updated_and_requires_existence() {
        let (_dir, store) = temp_store();
        let collection = collection_id(1);

        let created = store.create(&collection, "conv").await.unwrap();
        store
            .set_title(&collection, "conv", "A Title")
            .await
            .unwrap();

        let meta = store.load_meta(&collection, "conv").await.unwrap().unwrap();
        assert_eq!(meta.title, "A Title");
        assert_eq!(meta.updated, created.updated);

        assert!(matches!(
            store.set_title(&collection, "missing", "x").await,
            Err(MemoryStoreError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn touch_bumps_updated() {
        let (_dir, store) = temp_store();
        let collection = collection_id(1);

        let created = store.create(&collection, "conv").await.unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        store.touch(&collection, "conv").await.unwrap();

        let meta = store.load_meta(&collection, "conv").await.unwrap().unwrap();
        assert!(meta.updated > created.updated);
    }

    #[tokio::test]
    async fn listing_is_per_collection_sorted_and_titles_default() {
        let (_dir, store) = temp_store();
        let first = collection_id(1);
        let second = collection_id(2);

        store.create(&first, "older").await.unwrap();
        store
            .set_title(&first, "older", "Older title")
            .await
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        store.create(&first, "newer").await.unwrap();
        store.create(&second, "other").await.unwrap();

        let summaries = store.list_summaries(&first).await.unwrap();
        let ids: Vec<&str> = summaries.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["newer", "older"]);
        assert_eq!(summaries[0].title, DEFAULT_TITLE);
        assert_eq!(summaries[1].title, "Older title");

        let other = store.list_summaries(&second).await.unwrap();
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].id, "other");

        assert!(
            store
                .list_summaries(&collection_id(3))
                .await
                .unwrap()
                .is_empty(),
            "a missing collection directory lists empty"
        );
    }

    #[tokio::test]
    async fn corrupt_meta_fails_listing() {
        let (_dir, store) = temp_store();
        let collection = collection_id(1);
        store.create(&collection, "conv").await.unwrap();

        let meta_path = store
            .conversation_dir(&collection, "conv")
            .unwrap()
            .join("files")
            .join(META);
        std::fs::write(meta_path, b"{not json").unwrap();

        assert!(matches!(
            store.list_summaries(&collection).await,
            Err(MemoryStoreError::InvalidMetadata(_))
        ));
    }

    #[tokio::test]
    async fn corrupt_meta_is_invalid_and_absent_meta_is_none() {
        let (_dir, store) = temp_store();
        let collection = collection_id(1);
        store.create(&collection, "conv").await.unwrap();

        store
            .store(&collection, "conv")
            .unwrap()
            .write_file(META, b"{not json")
            .unwrap();

        assert!(matches!(
            store.load_meta(&collection, "conv").await,
            Err(MemoryStoreError::InvalidMetadata(_))
        ));
        assert!(store
            .load_meta(&collection, "missing")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn conversation_without_meta_fails_listing() {
        let (_dir, store) = temp_store();
        let collection = collection_id(1);
        std::fs::create_dir_all(store.collection_dir(&collection).unwrap().join("empty")).unwrap();

        assert!(matches!(
            store.list_summaries(&collection).await,
            Err(MemoryStoreError::InvalidMetadata(_))
        ));
    }

    #[tokio::test]
    async fn delete_removes_only_that_conversation_and_is_idempotent() {
        let (_dir, store) = temp_store();
        let collection = collection_id(1);
        let other = collection_id(2);

        store.create(&collection, "keep").await.unwrap();
        store.create(&collection, "gone").await.unwrap();
        store.create(&other, "gone").await.unwrap();
        save_messages(&store, &collection, "gone", vec![Message::user("hi")]).await;

        store.delete(&collection, "gone").await.unwrap();
        assert!(store
            .load_meta(&collection, "gone")
            .await
            .unwrap()
            .is_none());
        assert!(store.load(&collection, "gone").await.unwrap().is_empty());
        assert!(store
            .load_meta(&collection, "keep")
            .await
            .unwrap()
            .is_some());
        assert!(
            store.load_meta(&other, "gone").await.unwrap().is_some(),
            "the same id in another collection is untouched"
        );

        // Deleting again is fine.
        store.delete(&collection, "gone").await.unwrap();
    }

    #[tokio::test]
    async fn delete_collection_removes_only_that_collection() {
        let (_dir, store) = temp_store();
        let first = collection_id(1);
        let second = collection_id(2);

        store.create(&first, "conv").await.unwrap();
        store.create(&second, "conv").await.unwrap();

        store.delete_collection(&first).await.unwrap();
        assert!(store.list_summaries(&first).await.unwrap().is_empty());
        assert_eq!(store.list_summaries(&second).await.unwrap().len(), 1);

        // Idempotent.
        store.delete_collection(&first).await.unwrap();
    }

    #[tokio::test]
    async fn invalid_ids_error() {
        let (_dir, store) = temp_store();
        let collection = collection_id(1);

        assert!(matches!(
            store.create("not-a-namespace", "conv").await,
            Err(MemoryStoreError::InvalidCollection(_))
        ));
        assert!(matches!(
            store.create(&collection, "").await,
            Err(MemoryStoreError::InvalidConversationId(_))
        ));
        assert!(matches!(
            store.create(&collection, "..").await,
            Err(MemoryStoreError::InvalidConversationId(_))
        ));
        assert!(matches!(
            store.create(&collection, "nested/conv").await,
            Err(MemoryStoreError::InvalidConversationId(_))
        ));
    }

    #[tokio::test]
    async fn message_round_trip_with_tool_blocks() {
        let (_dir, store) = temp_store();
        let collection = collection_id(1);
        store.create(&collection, "conv").await.unwrap();
        let expected = messages_with_tools();
        save_messages(&store, &collection, "conv", expected.clone()).await;

        let loaded = store.load(&collection, "conv").await.unwrap();
        assert_eq!(loaded, expected);
    }

    #[tokio::test]
    async fn unknown_conversation_loads_empty() {
        let (_dir, store) = temp_store();
        let collection = collection_id(1);

        assert!(store.load(&collection, "nope").await.unwrap().is_empty());
        assert!(store
            .load_meta(&collection, "nope")
            .await
            .unwrap()
            .is_none());
    }
}
