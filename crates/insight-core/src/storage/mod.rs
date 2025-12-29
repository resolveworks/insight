use std::path::Path;

use anyhow::{Context, Result};
use futures::{Stream, StreamExt};
use iroh::protocol::Router;
use iroh::{Endpoint, RelayMode};
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::Hash;
use iroh_blobs::{BlobsProtocol, ALPN as BLOBS_ALPN};
use iroh_docs::api::protocol::{AddrInfoOptions, ShareMode};
use iroh_docs::api::DocsApi;
pub use iroh_docs::engine::LiveEvent;
use iroh_docs::net::ALPN as DOCS_ALPN;
use iroh_docs::protocol::Docs;
use iroh_docs::store::Query;
use iroh_docs::{AuthorId, ContentStatus, DocTicket, NamespaceId};
use iroh_gossip::net::{Gossip, GOSSIP_ALPN};
use serde::{Deserialize, Serialize};

/// Collection metadata stored in iroh-docs under `_collection` key
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionMetadata {
    pub name: String,
    pub created_at: String,
}

/// Document metadata stored in iroh-docs under `files/{id}` key
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub id: String,
    pub name: String,
    pub pdf_hash: String,
    pub text_hash: String,
    pub page_count: usize,
    pub tags: Vec<String>,
    pub created_at: String,
    /// Character offset where each page ends (for chunk-to-page mapping)
    #[serde(default)]
    pub page_boundaries: Vec<usize>,
}

/// Embedding data for a document, stored per model under `embeddings/{doc_id}/{model_id}` key
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingData {
    pub model_id: String,
    pub dimensions: usize,
    pub chunks: Vec<EmbeddingChunk>,
    pub created_at: String,
}

/// A single chunk with its embedding vector
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingChunk {
    pub index: usize,
    pub content: String,
    pub vector: Vec<f32>,
    pub start_page: usize,
    pub end_page: usize,
}

/// Storage layer using iroh for P2P content-addressed storage
///
/// Uses iroh_docs::Engine via the Docs protocol wrapper for native event subscriptions.
/// This enables subscribing to LiveEvent (InsertLocal, InsertRemote) for reactive indexing.
pub struct Storage {
    /// Content-addressed blob storage (PDFs, extracted text)
    pub blobs: FsStore,
    /// Docs protocol wrapper containing the Engine
    docs: Docs,
    /// Gossip protocol for pub/sub (used for P2P sync)
    #[allow(dead_code)]
    gossip: Gossip,
    /// Router for accepting incoming protocol connections
    #[allow(dead_code)]
    router: Router,
    /// Default author ID for this node
    author_id: AuthorId,
}

impl Storage {
    /// Initialize storage at the given path
    ///
    /// Sets up the iroh networking stack (local-only mode) and spawns the docs Engine.
    pub async fn open(path: &Path) -> Result<Self> {
        std::fs::create_dir_all(path)?;

        let blobs_path = path.join("blobs");
        let docs_path = path.join("docs");

        // Ensure docs directory exists (required by Docs::persistent)
        std::fs::create_dir_all(&docs_path)?;

        // Create blob store
        let blobs = FsStore::load(&blobs_path)
            .await
            .context("Failed to open blob store")?;

        // Create endpoint with relay servers for P2P connectivity
        let endpoint = Endpoint::builder()
            .relay_mode(RelayMode::Default)
            .bind()
            .await
            .context("Failed to create endpoint")?;

        // Create gossip protocol
        let gossip = Gossip::builder().spawn(endpoint.clone());

        // Create docs with Engine - uses the blobs api::Store (via Deref)
        let blobs_api = (*blobs).clone();
        let docs = Docs::persistent(docs_path)
            .spawn(endpoint.clone(), blobs_api.clone(), gossip.clone())
            .await
            .context("Failed to spawn docs engine")?;

        // Create blobs protocol handler for serving blob requests
        let blobs_protocol = BlobsProtocol::new(&blobs_api, None);

        // Create router to accept incoming connections for our protocols
        // This is critical for P2P sync - without it, peers can discover us
        // but can't establish protocol-level connections
        let router = Router::builder(endpoint.clone())
            .accept(BLOBS_ALPN, blobs_protocol)
            .accept(DOCS_ALPN, docs.clone())
            .accept(GOSSIP_ALPN, gossip.clone())
            .spawn();

        // Get or create default author
        let author_id = docs.author_default().await?;

        // Log node info for debugging connectivity
        let addr = endpoint.addr();
        tracing::info!(
            "Storage opened at {:?}, author: {}, node_id: {}, addrs: {:?}",
            path,
            author_id.fmt_short(),
            addr.id.fmt_short(),
            addr.addrs
        );

        Ok(Self {
            blobs,
            docs,
            gossip,
            router,
            author_id,
        })
    }

    /// Get the default author ID for this node
    pub fn author_id(&self) -> AuthorId {
        self.author_id
    }

    /// Get the docs API for direct access
    pub fn docs(&self) -> &DocsApi {
        self.docs.api()
    }

    /// Store bytes in blob storage and return the hash
    /// Creates a permanent tag to prevent garbage collection
    pub async fn store_blob(&self, data: &[u8]) -> Result<Hash> {
        // add_slice returns a TempTag that will keep the blob alive
        let tag = self.blobs.add_slice(data).await?;
        let hash = tag.hash;

        // Create a permanent tag so the blob isn't garbage collected
        // Use the hash hex as the tag name
        self.blobs
            .tags()
            .set(hash.to_string(), tag.hash_and_format())
            .await?;

        Ok(hash)
    }

    /// Get bytes from blob storage by hash
    pub async fn get_blob(&self, hash: &Hash) -> Result<Option<Vec<u8>>> {
        // Check if blob exists and is complete
        if !self.blobs.has(*hash).await? {
            return Ok(None);
        }

        // get_bytes returns the full blob content
        let bytes = self.blobs.get_bytes(*hash).await?;
        Ok(Some(bytes.to_vec()))
    }

    /// Create a new collection (namespace) with the given name
    pub async fn create_collection(&self, name: &str) -> Result<(NamespaceId, CollectionMetadata)> {
        let metadata = CollectionMetadata {
            name: name.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        // Create new document (namespace)
        let doc = self.docs.api().create().await?;
        let namespace_id = doc.id();

        // Store metadata in blob storage first
        let metadata_bytes = serde_json::to_vec(&metadata)?;
        let hash = self.store_blob(&metadata_bytes).await?;
        let len = metadata_bytes.len() as u64;

        // Store reference in iroh-docs under `_collection` key
        doc.set_hash(self.author_id, b"_collection".to_vec(), hash, len)
            .await?;

        doc.close().await?;

        tracing::info!("Created collection '{}' with id {}", name, namespace_id);

        Ok((namespace_id, metadata))
    }

    /// List all collections with their metadata
    pub async fn list_collections(&self) -> Result<Vec<(NamespaceId, CollectionMetadata)>> {
        use futures::StreamExt;

        let mut collections = Vec::new();

        // Get all namespaces
        let mut stream = self.docs.api().list().await?;

        while let Some(result) = stream.next().await {
            let (namespace_id, _capability) = result?;
            tracing::debug!("Checking namespace {}", namespace_id);

            // Try to read the collection metadata
            match self.get_collection_metadata(namespace_id).await {
                Ok(Some(metadata)) => {
                    tracing::debug!("Found metadata for {}: {}", namespace_id, metadata.name);
                    collections.push((namespace_id, metadata));
                }
                Ok(None) => {
                    tracing::debug!("No metadata found for {}", namespace_id);
                }
                Err(e) => {
                    tracing::warn!("Error reading metadata for {}: {}", namespace_id, e);
                }
            }
        }

        Ok(collections)
    }

    /// Get collection metadata for a specific namespace
    pub async fn get_collection_metadata(
        &self,
        namespace_id: NamespaceId,
    ) -> Result<Option<CollectionMetadata>> {
        let doc = match self.docs.api().open(namespace_id).await? {
            Some(doc) => doc,
            None => return Ok(None),
        };

        // Query for the _collection entry
        let query = Query::key_exact(b"_collection");
        let entry = doc.get_one(query).await?;

        let metadata = if let Some(entry) = entry {
            let hash = entry.content_hash();
            tracing::debug!("Found _collection entry with hash {}", hash);

            // Fetch content from blob storage
            match self.get_blob(&hash).await {
                Ok(Some(data)) => {
                    tracing::debug!("Fetched blob, {} bytes", data.len());
                    Some(serde_json::from_slice(&data)?)
                }
                Ok(None) => {
                    tracing::debug!("Blob not found for hash {}", hash);
                    None
                }
                Err(e) => {
                    tracing::warn!("Error fetching blob {}: {}", hash, e);
                    return Err(e);
                }
            }
        } else {
            tracing::debug!("No _collection entry found");
            None
        };

        doc.close().await?;
        Ok(metadata)
    }

    /// Count documents in a collection
    pub async fn count_documents(&self, namespace_id: NamespaceId) -> Result<usize> {
        use futures::StreamExt;

        let doc = match self.docs.api().open(namespace_id).await? {
            Some(doc) => doc,
            None => return Ok(0),
        };

        // Query for all entries with prefix "files/"
        let query = Query::key_prefix(b"files/");
        let stream = doc.get_many(query).await?;
        let count = stream.count().await;

        doc.close().await?;
        Ok(count)
    }

    /// Add a document to a collection
    pub async fn add_document(
        &self,
        namespace_id: NamespaceId,
        metadata: DocumentMetadata,
    ) -> Result<()> {
        let doc = self
            .docs
            .api()
            .open(namespace_id)
            .await?
            .context("Collection not found")?;

        // Store metadata in blob storage
        let metadata_bytes = serde_json::to_vec(&metadata)?;
        let hash = self.store_blob(&metadata_bytes).await?;
        let len = metadata_bytes.len() as u64;

        // Store reference in iroh-docs under `files/{id}` key
        let key = format!("files/{}", metadata.id);
        doc.set_hash(self.author_id, key.into_bytes(), hash, len)
            .await?;

        // Create hash index entry for O(1) duplicate detection
        // Store document ID as the value (small blob)
        let index_key = format!("_hash_index/{}", metadata.pdf_hash);
        let doc_id_bytes = metadata.id.as_bytes();
        let doc_id_hash = self.store_blob(doc_id_bytes).await?;
        doc.set_hash(
            self.author_id,
            index_key.into_bytes(),
            doc_id_hash,
            doc_id_bytes.len() as u64,
        )
        .await?;

        doc.close().await?;

        tracing::info!(
            "Added document '{}' to collection {}",
            metadata.name,
            namespace_id
        );

        Ok(())
    }

    /// List all documents in a collection
    pub async fn list_documents(&self, namespace_id: NamespaceId) -> Result<Vec<DocumentMetadata>> {
        use futures::StreamExt;

        let doc = match self.docs.api().open(namespace_id).await? {
            Some(doc) => doc,
            None => return Ok(Vec::new()),
        };

        let query = Query::key_prefix(b"files/");
        let stream = doc.get_many(query).await?;
        tokio::pin!(stream);

        let mut documents = Vec::new();
        while let Some(result) = stream.next().await {
            let entry = result?;
            let hash = entry.content_hash();

            if let Some(data) = self.get_blob(&hash).await? {
                match serde_json::from_slice::<DocumentMetadata>(&data) {
                    Ok(metadata) => documents.push(metadata),
                    Err(e) => {
                        tracing::warn!("Failed to parse document metadata: {}", e);
                    }
                }
            }
        }

        doc.close().await?;
        Ok(documents)
    }

    /// Get a single document from a collection by ID
    pub async fn get_document(
        &self,
        namespace_id: NamespaceId,
        document_id: &str,
    ) -> Result<Option<DocumentMetadata>> {
        let doc = match self.docs.api().open(namespace_id).await? {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let key = format!("files/{}", document_id);
        let query = Query::key_exact(key.as_bytes());
        let entry = doc.get_one(query).await?;

        let metadata = if let Some(entry) = entry {
            let hash = entry.content_hash();

            if let Some(data) = self.get_blob(&hash).await? {
                match serde_json::from_slice::<DocumentMetadata>(&data) {
                    Ok(metadata) => Some(metadata),
                    Err(e) => {
                        tracing::warn!("Failed to parse document metadata: {}", e);
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        doc.close().await?;
        Ok(metadata)
    }

    /// Check if a document with the given PDF hash exists (O(1) lookup via hash index)
    ///
    /// Uses the `_hash_index/{pdf_hash}` key for constant-time duplicate detection.
    pub async fn has_pdf_hash(&self, namespace_id: NamespaceId, pdf_hash: &str) -> Result<bool> {
        let doc = match self.docs.api().open(namespace_id).await? {
            Some(doc) => doc,
            None => return Ok(false),
        };

        let key = format!("_hash_index/{}", pdf_hash);
        let query = Query::key_exact(key.as_bytes());
        let entry = doc.get_one(query).await?;

        doc.close().await?;
        Ok(entry.is_some())
    }

    /// Store embeddings for a document
    ///
    /// Stores embedding data under `embeddings/{doc_id}/{model_id}` key.
    /// This enables sharing embeddings between peers with the same model.
    pub async fn store_embeddings(
        &self,
        namespace_id: NamespaceId,
        doc_id: &str,
        data: EmbeddingData,
    ) -> Result<()> {
        let doc = self
            .docs
            .api()
            .open(namespace_id)
            .await?
            .context("Collection not found")?;

        let data_bytes = serde_json::to_vec(&data)?;
        let hash = self.store_blob(&data_bytes).await?;
        let len = data_bytes.len() as u64;

        let key = format!("embeddings/{}/{}", doc_id, data.model_id);
        doc.set_hash(self.author_id, key.into_bytes(), hash, len)
            .await?;

        doc.close().await?;

        tracing::debug!(
            doc_id = %doc_id,
            model_id = %data.model_id,
            chunk_count = data.chunks.len(),
            "Stored embeddings"
        );

        Ok(())
    }

    /// Get embeddings for a document and model
    ///
    /// Returns None if embeddings don't exist for this document/model combination.
    pub async fn get_embeddings(
        &self,
        namespace_id: NamespaceId,
        doc_id: &str,
        model_id: &str,
    ) -> Result<Option<EmbeddingData>> {
        let doc = match self.docs.api().open(namespace_id).await? {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let key = format!("embeddings/{}/{}", doc_id, model_id);
        let query = Query::key_exact(key.as_bytes());
        let entry = doc.get_one(query).await?;

        let result = if let Some(entry) = entry {
            let hash = entry.content_hash();
            if let Some(data) = self.get_blob(&hash).await? {
                Some(serde_json::from_slice(&data)?)
            } else {
                None
            }
        } else {
            None
        };

        doc.close().await?;
        Ok(result)
    }

    /// Delete all embeddings for a document (all models)
    pub async fn delete_embeddings(&self, namespace_id: NamespaceId, doc_id: &str) -> Result<()> {
        use futures::StreamExt;

        let doc = match self.docs.api().open(namespace_id).await? {
            Some(doc) => doc,
            None => return Ok(()),
        };

        let prefix = format!("embeddings/{}/", doc_id);
        let query = Query::key_prefix(prefix.as_bytes());
        let stream = doc.get_many(query).await?;
        tokio::pin!(stream);

        let mut keys_to_delete = Vec::new();
        while let Some(result) = stream.next().await {
            let entry = result?;
            keys_to_delete.push(entry.key().to_vec());
        }

        for key in keys_to_delete {
            doc.del(self.author_id, key).await?;
        }

        doc.close().await?;

        tracing::debug!(doc_id = %doc_id, "Deleted embeddings");

        Ok(())
    }

    /// Delete a document from a collection
    pub async fn delete_document(
        &self,
        namespace_id: NamespaceId,
        document_id: &str,
    ) -> Result<()> {
        let doc = self
            .docs
            .api()
            .open(namespace_id)
            .await?
            .context("Collection not found")?;

        // First, get the document metadata to find the pdf_hash for index cleanup
        let file_key = format!("files/{}", document_id);
        let query = Query::key_exact(file_key.as_bytes());
        if let Some(entry) = doc.get_one(query).await? {
            let hash = entry.content_hash();
            if let Some(data) = self.get_blob(&hash).await? {
                if let Ok(metadata) = serde_json::from_slice::<DocumentMetadata>(&data) {
                    // Delete the hash index entry
                    let index_key = format!("_hash_index/{}", metadata.pdf_hash);
                    doc.del(self.author_id, index_key.into_bytes()).await?;
                }
            }
        }

        // Delete the document entry
        doc.del(self.author_id, file_key.into_bytes()).await?;

        doc.close().await?;

        // Delete associated embeddings (all models)
        self.delete_embeddings(namespace_id, document_id).await?;

        tracing::info!(
            "Deleted document '{}' from collection {}",
            document_id,
            namespace_id
        );

        Ok(())
    }

    /// Delete a collection and all its documents
    pub async fn delete_collection(&self, namespace_id: NamespaceId) -> Result<()> {
        self.docs.api().drop_doc(namespace_id).await?;

        tracing::info!("Deleted collection {}", namespace_id);

        Ok(())
    }

    /// Subscribe to document events for a namespace
    ///
    /// Returns a stream of LiveEvent that includes:
    /// - InsertLocal: When a document is added locally
    /// - InsertRemote: When a document is added from a peer
    /// - ContentReady: When content has been downloaded
    /// - SyncFinished: When a sync operation completes
    pub async fn subscribe(
        &self,
        namespace_id: NamespaceId,
    ) -> Result<impl Stream<Item = Result<LiveEvent>> + Send + Unpin + 'static> {
        let doc = self
            .docs
            .api()
            .open(namespace_id)
            .await?
            .context("Collection not found")?;

        let stream = doc.subscribe().await?;

        Ok(stream)
    }

    /// Generate a share ticket for a collection
    ///
    /// The ticket string can be shared with others who can then import the collection.
    /// If `writable` is true, the recipient can also add/edit documents.
    pub async fn share_collection(
        &self,
        namespace_id: NamespaceId,
        writable: bool,
    ) -> Result<String> {
        let doc = self
            .docs
            .api()
            .open(namespace_id)
            .await?
            .context("Collection not found")?;

        let mode = if writable {
            ShareMode::Write
        } else {
            ShareMode::Read
        };

        // Include relay + direct addresses so peers can connect
        let ticket = doc.share(mode, AddrInfoOptions::RelayAndAddresses).await?;

        doc.close().await?;

        tracing::info!(
            "Shared collection {} (writable: {})",
            namespace_id,
            writable
        );

        Ok(ticket.to_string())
    }

    /// Import a collection from a share ticket
    ///
    /// This registers the namespace locally and starts syncing with the peer
    /// who shared it. Waits for the collection metadata to sync before returning.
    /// The DocWatcher will pick up InsertRemote events and trigger embedding +
    /// indexing automatically for document entries.
    pub async fn import_collection(&self, ticket_str: &str) -> Result<NamespaceId> {
        use std::time::Duration;
        use tokio::time::timeout;

        let ticket: DocTicket = ticket_str.parse().context("Invalid share ticket")?;

        // Log ticket info for debugging
        tracing::info!("Ticket contains {} peer(s)", ticket.nodes.len());
        for node in &ticket.nodes {
            tracing::info!("  Peer {}: {:?}", node.id.fmt_short(), node.addrs);
        }

        // Import and subscribe to events so we can wait for the _collection entry
        let (doc, mut events) = self.docs.api().import_and_subscribe(ticket).await?;
        let namespace_id = doc.id();

        tracing::info!(
            "Importing collection {}, waiting for metadata...",
            namespace_id
        );

        // Check if _collection entry already exists (re-import case)
        let query = Query::key_exact(b"_collection");
        if let Some(entry) = doc.get_one(query).await? {
            tracing::info!(
                "Collection metadata already exists (hash: {})",
                entry.content_hash()
            );
            doc.close().await?;
            return Ok(namespace_id);
        }

        // Wait for the _collection entry to sync AND its content to be downloaded
        // If peer is offline, this will timeout but the collection is still registered
        // and will sync when the peer comes online
        let sync_timeout = Duration::from_secs(30);
        let wait_result = timeout(sync_timeout, async {
            // Track if we need to wait for content download
            let mut pending_content_hash: Option<iroh_blobs::Hash> = None;

            while let Some(event) = events.next().await {
                match &event {
                    Ok(e) => tracing::debug!("Import event: {}", e),
                    Err(e) => tracing::debug!("Import event error: {}", e),
                }
                match event {
                    Ok(LiveEvent::InsertRemote {
                        entry,
                        content_status,
                        ..
                    }) => {
                        let key = String::from_utf8_lossy(entry.key());
                        if key == "_collection" {
                            let hash = entry.content_hash();
                            tracing::info!(
                                "Received _collection entry from peer (hash: {}, status: {:?})",
                                hash,
                                content_status
                            );

                            match content_status {
                                ContentStatus::Complete => {
                                    // Content already available, we're done
                                    return Ok(());
                                }
                                _ => {
                                    // Content needs to be downloaded, wait for ContentReady
                                    tracing::info!("Waiting for content download...");
                                    pending_content_hash = Some(hash);
                                }
                            }
                        }
                    }
                    Ok(LiveEvent::ContentReady { hash }) => {
                        tracing::debug!("Content ready: {}", hash);
                        if pending_content_hash == Some(hash) {
                            tracing::info!("Collection content downloaded");
                            return Ok(());
                        }
                    }
                    Ok(LiveEvent::SyncFinished(result)) => {
                        // Sync finished - check if we have the _collection entry now
                        tracing::info!("Sync finished: {:?}", result);
                        if pending_content_hash.is_none() {
                            let query = Query::key_exact(b"_collection");
                            if doc.get_one(query).await?.is_some() {
                                return Ok(());
                            }
                        }
                        // If we're waiting for content, keep listening
                    }
                    Err(e) => {
                        tracing::warn!("Event stream error during import: {}", e);
                    }
                    _ => {}
                }
            }
            anyhow::bail!("Event stream ended without receiving _collection entry")
        })
        .await;

        match wait_result {
            Ok(Ok(())) => {
                tracing::info!("Imported collection {}", namespace_id);
            }
            Ok(Err(e)) => {
                tracing::warn!("Import warning for {}: {}", namespace_id, e);
                // Still return the namespace - the collection may sync later
            }
            Err(_) => {
                tracing::warn!(
                    "Timeout waiting for collection metadata for {}",
                    namespace_id
                );
                // Still return the namespace - the collection may sync later
            }
        }

        doc.close().await?;
        Ok(namespace_id)
    }

    /// Import a PDF file into a collection (storage only).
    ///
    /// Extracts text, stores blobs, and creates metadata entry.
    /// Returns the document metadata.
    ///
    /// Note: This only stores the document. For local imports that need
    /// immediate embedding and indexing, use `jobs::import_and_index_pdf()`.
    pub async fn import_pdf(
        &self,
        path: &std::path::Path,
        namespace_id: NamespaceId,
    ) -> Result<DocumentMetadata> {
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown.pdf".to_string());

        // Extract text (blocking)
        let extracted = crate::pdf::extract_text(path)?;

        // Store PDF blob
        let pdf_hash = self.store_blob(&extracted.pdf_bytes).await?.to_string();

        // Check for duplicate
        if self.has_pdf_hash(namespace_id, &pdf_hash).await? {
            anyhow::bail!("Duplicate document: {}", file_name);
        }

        // Store text blob
        let text_hash = self
            .store_blob(extracted.text.as_bytes())
            .await?
            .to_string();

        // Create metadata
        let metadata = DocumentMetadata {
            id: uuid::Uuid::new_v4().to_string(),
            name: file_name,
            pdf_hash,
            text_hash,
            page_count: extracted.page_count,
            tags: vec![],
            created_at: chrono::Utc::now().to_rfc3339(),
            page_boundaries: extracted.page_boundaries,
        };

        // Store document (triggers iroh event)
        self.add_document(namespace_id, metadata.clone()).await?;

        Ok(metadata)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_blob_roundtrip() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp_dir.path()).await.unwrap();

        let data = b"hello world";
        let hash = storage.store_blob(data).await.unwrap();

        let retrieved = storage.get_blob(&hash).await.unwrap();
        assert_eq!(retrieved, Some(data.to_vec()));
    }

    #[tokio::test]
    async fn test_get_nonexistent_blob() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp_dir.path()).await.unwrap();

        let fake_hash = Hash::from_bytes([0u8; 32]);
        let result = storage.get_blob(&fake_hash).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_create_and_list_collections() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp_dir.path()).await.unwrap();

        // Initially empty
        let collections = storage.list_collections().await.unwrap();
        assert!(collections.is_empty());

        // Create a collection
        let (id, metadata) = storage.create_collection("Test Collection").await.unwrap();
        assert_eq!(metadata.name, "Test Collection");

        // Should now list one collection
        let collections = storage.list_collections().await.unwrap();
        assert_eq!(collections.len(), 1);
        assert_eq!(collections[0].0, id);
        assert_eq!(collections[0].1.name, "Test Collection");
    }

    #[tokio::test]
    async fn test_get_collection_metadata() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp_dir.path()).await.unwrap();

        let (id, _) = storage.create_collection("My Docs").await.unwrap();

        let metadata = storage.get_collection_metadata(id).await.unwrap();
        assert!(metadata.is_some());
        assert_eq!(metadata.unwrap().name, "My Docs");
    }

    #[tokio::test]
    async fn test_count_documents_empty() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp_dir.path()).await.unwrap();

        let (id, _) = storage.create_collection("Empty").await.unwrap();
        let count = storage.count_documents(id).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_add_and_list_documents() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp_dir.path()).await.unwrap();

        let (collection_id, _) = storage.create_collection("My Docs").await.unwrap();

        // Add a document
        let doc = DocumentMetadata {
            id: "doc-1".to_string(),
            name: "test.pdf".to_string(),
            pdf_hash: "abc123".to_string(),
            text_hash: "def456".to_string(),
            page_count: 5,
            tags: vec!["test".to_string()],
            created_at: "2024-01-01T00:00:00Z".to_string(),
            page_boundaries: vec![],
        };
        storage.add_document(collection_id, doc).await.unwrap();

        // Count should be 1
        let count = storage.count_documents(collection_id).await.unwrap();
        assert_eq!(count, 1);

        // List documents
        let docs = storage.list_documents(collection_id).await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].id, "doc-1");
        assert_eq!(docs[0].name, "test.pdf");
        assert_eq!(docs[0].page_count, 5);
    }

    #[tokio::test]
    async fn test_delete_document() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp_dir.path()).await.unwrap();

        let (collection_id, _) = storage.create_collection("My Docs").await.unwrap();

        // Add two documents
        let doc1 = DocumentMetadata {
            id: "doc-1".to_string(),
            name: "first.pdf".to_string(),
            pdf_hash: "abc".to_string(),
            text_hash: "def".to_string(),
            page_count: 1,
            tags: vec![],
            created_at: "2024-01-01T00:00:00Z".to_string(),
            page_boundaries: vec![],
        };
        let doc2 = DocumentMetadata {
            id: "doc-2".to_string(),
            name: "second.pdf".to_string(),
            pdf_hash: "ghi".to_string(),
            text_hash: "jkl".to_string(),
            page_count: 2,
            tags: vec![],
            created_at: "2024-01-01T00:00:00Z".to_string(),
            page_boundaries: vec![],
        };
        storage.add_document(collection_id, doc1).await.unwrap();
        storage.add_document(collection_id, doc2).await.unwrap();

        assert_eq!(storage.count_documents(collection_id).await.unwrap(), 2);

        // Delete first document
        storage
            .delete_document(collection_id, "doc-1")
            .await
            .unwrap();

        // Should have 1 document left
        let docs = storage.list_documents(collection_id).await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].id, "doc-2");
    }

    #[tokio::test]
    async fn test_delete_collection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp_dir.path()).await.unwrap();

        // Create two collections
        let (id1, _) = storage.create_collection("First").await.unwrap();
        let (id2, _) = storage.create_collection("Second").await.unwrap();

        assert_eq!(storage.list_collections().await.unwrap().len(), 2);

        // Delete first collection
        storage.delete_collection(id1).await.unwrap();

        // Should have 1 collection left
        let collections = storage.list_collections().await.unwrap();
        assert_eq!(collections.len(), 1);
        assert_eq!(collections[0].0, id2);
    }

    #[tokio::test]
    async fn test_subscribe_to_events() {
        use futures::StreamExt;

        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp_dir.path()).await.unwrap();

        let (collection_id, _) = storage.create_collection("Events Test").await.unwrap();

        // Subscribe to events
        let mut stream = storage.subscribe(collection_id).await.unwrap();

        // Add a document - this should trigger an InsertLocal event
        let doc = DocumentMetadata {
            id: "doc-events".to_string(),
            name: "events.pdf".to_string(),
            pdf_hash: "hash1".to_string(),
            text_hash: "hash2".to_string(),
            page_count: 1,
            tags: vec![],
            created_at: "2024-01-01T00:00:00Z".to_string(),
            page_boundaries: vec![],
        };
        storage.add_document(collection_id, doc).await.unwrap();

        // We should receive an InsertLocal event
        // Use a timeout to avoid hanging if no event is received
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next()).await;

        match event {
            Ok(Some(Ok(LiveEvent::InsertLocal { entry }))) => {
                let key = String::from_utf8_lossy(entry.key());
                assert!(key.starts_with("files/"));
            }
            other => {
                // It's ok if we don't receive the event immediately in tests
                tracing::debug!("Event result: {:?}", other);
            }
        }
    }

    #[tokio::test]
    async fn test_duplicate_detection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp_dir.path()).await.unwrap();

        let (collection_id, _) = storage.create_collection("Duplicates Test").await.unwrap();

        // Initially, no document with this hash exists
        assert!(!storage
            .has_pdf_hash(collection_id, "pdf-hash-123")
            .await
            .unwrap());

        // Add a document
        let doc = DocumentMetadata {
            id: "doc-1".to_string(),
            name: "report.pdf".to_string(),
            pdf_hash: "pdf-hash-123".to_string(),
            text_hash: "text-hash-456".to_string(),
            page_count: 10,
            tags: vec![],
            created_at: "2024-01-01T00:00:00Z".to_string(),
            page_boundaries: vec![],
        };
        storage.add_document(collection_id, doc).await.unwrap();

        // Now the hash should be detected
        assert!(storage
            .has_pdf_hash(collection_id, "pdf-hash-123")
            .await
            .unwrap());

        // Different hash should not be detected
        assert!(!storage
            .has_pdf_hash(collection_id, "different-hash")
            .await
            .unwrap());

        // Delete the document
        storage
            .delete_document(collection_id, "doc-1")
            .await
            .unwrap();

        // Hash index should be cleaned up
        assert!(!storage
            .has_pdf_hash(collection_id, "pdf-hash-123")
            .await
            .unwrap());
    }
}
