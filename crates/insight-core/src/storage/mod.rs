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

use crate::text::{Extraction, ParserId, Unit, UnitKind};

// =============================================================================
// Key Structure Constants
// =============================================================================
//
// Document entries follow the pattern: files/{doc_id}/{part}
// where part is one of: meta, source, extractions/{parser_key}.
// A document's id is the BLAKE3 hash iroh computed for its source blob.
//
// This structure ensures iroh-docs syncs all parts automatically.

/// Key for collection metadata
pub const COLLECTION_KEY: &[u8] = b"_collection";

/// Prefix for all document entries
const FILES_PREFIX: &str = "files/";

/// Suffix for document metadata entries
const META_SUFFIX: &str = "/meta";

/// Suffix for document source file entries
const SOURCE_SUFFIX: &str = "/source";

/// Segment for a document's extraction entries
const EXTRACTIONS_SEGMENT: &str = "/extractions/";

/// Build the key for a document's metadata entry
#[inline]
pub fn doc_meta_key(doc_id: &str) -> String {
    format!("{}{}{}", FILES_PREFIX, doc_id, META_SUFFIX)
}

/// Build the key for a document's source file entry
#[inline]
pub fn doc_source_key(doc_id: &str) -> String {
    format!("{}{}{}", FILES_PREFIX, doc_id, SOURCE_SUFFIX)
}

/// Build the key for one of a document's extraction entries
#[inline]
pub fn doc_extraction_key(doc_id: &str, parser: &ParserId) -> String {
    format!(
        "{}{}{}{}",
        FILES_PREFIX,
        doc_id,
        EXTRACTIONS_SEGMENT,
        parser.key()
    )
}

/// Check if a key is a document metadata entry
#[inline]
pub fn is_doc_meta_key(key: &str) -> bool {
    key.starts_with(FILES_PREFIX) && key.ends_with(META_SUFFIX)
}

/// Extract the document ID from a files/{id}/meta key
#[inline]
pub fn extract_doc_id(key: &str) -> Option<&str> {
    key.strip_prefix(FILES_PREFIX)
        .and_then(|s| s.strip_suffix(META_SUFFIX))
}

// =============================================================================
// Data Structures
// =============================================================================

/// Collection metadata stored in iroh-docs under `_collection` key
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionMetadata {
    pub name: String,
    pub created_at: String,
}

/// Document metadata stored in iroh-docs under `files/{id}/meta` key
///
/// The actual content is stored in separate entries:
/// - `files/{id}/source` - original file bytes (PDF, etc.)
/// - `files/{id}/extractions/{parser_key}` - one extraction per parser
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub id: String,
    pub name: String,
    /// MIME type or file extension (e.g., "application/pdf", "pdf")
    #[serde(default = "default_file_type")]
    pub file_type: String,
    /// The extraction currently indexed for this document, if any.
    #[serde(default)]
    pub extraction: Option<ParserId>,
    pub tags: Vec<String>,
    pub created_at: String,
}

fn default_file_type() -> String {
    "application/pdf".to_string()
}

/// An extraction record stored under `files/{id}/extractions/{parser_key}`.
///
/// Holds the text blob hash and the source unit mapping; the text itself is
/// a separate blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExtractionRecord {
    parser: ParserId,
    /// Hex-encoded BLAKE3 hash of the extraction text blob.
    text_hash: String,
    unit_kind: Option<UnitKind>,
    units: Vec<Unit>,
}

impl ExtractionRecord {
    fn page_count(&self) -> Option<usize> {
        (self.unit_kind == Some(UnitKind::Page)).then_some(self.units.len())
    }
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
        let endpoint = Endpoint::builder(iroh::endpoint::presets::N0)
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

    /// Count documents in a collection.
    ///
    /// Counts only `files/*/meta` entries (one per document).
    pub async fn count_documents(&self, namespace_id: NamespaceId) -> Result<usize> {
        use futures::StreamExt;

        let doc = match self.docs.api().open(namespace_id).await? {
            Some(doc) => doc,
            None => return Ok(0),
        };

        // Query for all entries with prefix "files/" and count only /meta entries
        let query = Query::key_prefix(b"files/");
        let stream = doc.get_many(query).await?;
        tokio::pin!(stream);

        let mut count = 0;
        while let Some(result) = stream.next().await {
            if let Ok(entry) = result {
                let key = String::from_utf8_lossy(entry.key());
                if key.ends_with("/meta") {
                    count += 1;
                }
            }
        }

        doc.close().await?;
        Ok(count)
    }

    /// Add a document's immutable source to a collection.
    ///
    /// Stores two entries:
    /// - `files/{id}/meta` - document metadata (JSON)
    /// - `files/{id}/source` - original file bytes
    ///
    /// The document's id is the BLAKE3 hash iroh computes for the source, so
    /// a source already present in the collection is the same document and
    /// adding it again throws.
    pub async fn add_document(
        &self,
        namespace_id: NamespaceId,
        name: &str,
        file_type: &str,
        source_content: &[u8],
    ) -> Result<DocumentMetadata> {
        let source_hash = self.store_blob(source_content).await?;
        let metadata = DocumentMetadata {
            id: source_hash.to_hex(),
            name: name.to_string(),
            file_type: file_type.to_string(),
            extraction: None,
            tags: Vec::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        let doc = self
            .docs
            .api()
            .open(namespace_id)
            .await?
            .context("Collection not found")?;

        let meta_key = doc_meta_key(&metadata.id);
        if doc
            .get_one(Query::key_exact(meta_key.as_bytes()))
            .await?
            .is_some()
        {
            anyhow::bail!("Duplicate document: {name}");
        }

        self.store_meta_inner(&doc, &metadata.id, &metadata).await?;

        doc.set_hash(
            self.author_id,
            doc_source_key(&metadata.id).into_bytes(),
            source_hash,
            source_content.len() as u64,
        )
        .await?;

        doc.close().await?;

        tracing::info!(
            "Added document '{}' to collection {}",
            metadata.name,
            namespace_id
        );

        Ok(metadata)
    }

    /// List all documents in a collection
    ///
    /// Queries for all `files/*/meta` entries and returns their metadata.
    pub async fn list_documents(&self, namespace_id: NamespaceId) -> Result<Vec<DocumentMetadata>> {
        use futures::StreamExt;

        // An unknown namespace is "no documents", not an error. This iroh
        // version signals it inconsistently — sometimes `Ok(None)`, sometimes
        // an error whose typed `OpenError::NotFound` does not survive the
        // in-process RPC boundary and arrives as an opaque error carrying
        // only the display string — so match on the message.
        let doc = match self.docs.api().open(namespace_id).await {
            Ok(Some(doc)) => doc,
            Ok(None) => return Ok(Vec::new()),
            Err(e) if e.to_string() == "Replica not found" => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };

        // Query for all entries - we'll filter for /meta suffix
        let query = Query::key_prefix(b"files/");
        let stream = doc.get_many(query).await?;
        tokio::pin!(stream);

        let mut documents = Vec::new();
        while let Some(result) = stream.next().await {
            let entry = result?;
            let key = String::from_utf8_lossy(entry.key());

            // Only process /meta entries
            if !key.ends_with("/meta") {
                continue;
            }

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

    /// Get a single document's metadata from a collection by ID
    pub async fn get_document(
        &self,
        namespace_id: NamespaceId,
        document_id: &str,
    ) -> Result<Option<DocumentMetadata>> {
        let doc = match self.docs.api().open(namespace_id).await? {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let key = doc_meta_key(document_id);
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

    /// Write an extraction for a document without changing which extraction
    /// is canonical.
    pub async fn write_extraction(
        &self,
        namespace_id: NamespaceId,
        doc_id: &str,
        extraction: &Extraction,
    ) -> Result<()> {
        let doc = self
            .docs
            .api()
            .open(namespace_id)
            .await?
            .context("Collection not found")?;

        let text_bytes = extraction.text.as_bytes();
        let text_hash = self.store_blob(text_bytes).await?;

        let record = ExtractionRecord {
            parser: extraction.parser.clone(),
            text_hash: text_hash.to_hex(),
            unit_kind: extraction.unit_kind,
            units: extraction.units.clone(),
        };
        let record_bytes = serde_json::to_vec(&record)?;
        let record_hash = self.store_blob(&record_bytes).await?;

        let key = doc_extraction_key(doc_id, &extraction.parser);
        doc.set_hash(
            self.author_id,
            key.into_bytes(),
            record_hash,
            record_bytes.len() as u64,
        )
        .await?;

        doc.close().await?;
        Ok(())
    }

    /// Make one of a document's extractions canonical.
    ///
    /// The extraction entry must exist; selecting an unknown parser throws.
    pub async fn set_canonical_extraction(
        &self,
        namespace_id: NamespaceId,
        doc_id: &str,
        parser: &ParserId,
    ) -> Result<()> {
        let mut metadata = self
            .get_document(namespace_id, doc_id)
            .await?
            .context("Document not found")?;

        let doc = self
            .docs
            .api()
            .open(namespace_id)
            .await?
            .context("Collection not found")?;

        let key = doc_extraction_key(doc_id, parser);
        let entry = doc.get_one(Query::key_exact(key.as_bytes())).await?;
        if entry.is_none() {
            doc.close().await?;
            anyhow::bail!(
                "Extraction {} not found for document {}",
                parser.key(),
                doc_id
            );
        }

        metadata.extraction = Some(parser.clone());
        self.store_meta_inner(&doc, doc_id, &metadata).await?;
        doc.close().await?;
        Ok(())
    }

    /// Read one of a document's extractions.
    ///
    /// Returns `Ok(None)` when the extraction entry is absent or its text blob
    /// is not available locally yet (sync in flight).
    pub async fn get_extraction(
        &self,
        namespace_id: NamespaceId,
        doc_id: &str,
        parser: &ParserId,
    ) -> Result<Option<Extraction>> {
        let record = match self
            .get_extraction_record(namespace_id, doc_id, parser)
            .await?
        {
            Some(record) => record,
            None => return Ok(None),
        };

        let text_hash = record
            .text_hash
            .parse::<Hash>()
            .context("Invalid extraction text hash")?;
        let text_bytes = match self.get_blob(&text_hash).await? {
            Some(bytes) => bytes,
            None => return Ok(None),
        };
        let text = String::from_utf8(text_bytes).context("Extraction text is not valid UTF-8")?;

        Ok(Some(Extraction {
            parser: record.parser,
            text,
            unit_kind: record.unit_kind,
            units: record.units,
        }))
    }

    /// Read a document's canonical extraction, if one is selected.
    pub async fn get_canonical_extraction(
        &self,
        namespace_id: NamespaceId,
        doc_id: &str,
    ) -> Result<Option<Extraction>> {
        let metadata = match self.get_document(namespace_id, doc_id).await? {
            Some(metadata) => metadata,
            None => return Ok(None),
        };
        let Some(parser) = metadata.extraction else {
            return Ok(None);
        };
        self.get_extraction(namespace_id, doc_id, &parser).await
    }

    /// Number of pages in a document's canonical extraction.
    pub async fn page_count(
        &self,
        namespace_id: NamespaceId,
        doc_id: &str,
    ) -> Result<Option<usize>> {
        let metadata = match self.get_document(namespace_id, doc_id).await? {
            Some(metadata) => metadata,
            None => return Ok(None),
        };
        let Some(parser) = metadata.extraction else {
            return Ok(None);
        };
        Ok(self
            .get_extraction_record(namespace_id, doc_id, &parser)
            .await?
            .and_then(|record| record.page_count()))
    }

    /// Read an extraction record without loading its text blob.
    async fn get_extraction_record(
        &self,
        namespace_id: NamespaceId,
        doc_id: &str,
        parser: &ParserId,
    ) -> Result<Option<ExtractionRecord>> {
        let doc = match self.docs.api().open(namespace_id).await? {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let key = doc_extraction_key(doc_id, parser);
        let entry = doc.get_one(Query::key_exact(key.as_bytes())).await?;
        let record = if let Some(entry) = entry {
            match self.get_blob(&entry.content_hash()).await? {
                Some(bytes) => Some(serde_json::from_slice(&bytes)?),
                None => None,
            }
        } else {
            None
        };

        doc.close().await?;
        Ok(record)
    }

    /// Get a document's original source file
    pub async fn get_document_source(
        &self,
        namespace_id: NamespaceId,
        document_id: &str,
    ) -> Result<Option<Vec<u8>>> {
        let doc = match self.docs.api().open(namespace_id).await? {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let key = doc_source_key(document_id);
        let query = Query::key_exact(key.as_bytes());
        let entry = doc.get_one(query).await?;

        let source = if let Some(entry) = entry {
            self.get_blob(&entry.content_hash()).await?
        } else {
            None
        };

        doc.close().await?;
        Ok(source)
    }

    /// Delete a document from a collection
    ///
    /// Deletes the whole `files/{id}/` prefix in one tombstone: meta, source,
    /// extractions and anything added later.
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

        let prefix = format!("{}{}/", FILES_PREFIX, document_id);
        doc.del(self.author_id, prefix).await?;

        doc.close().await?;

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
    /// The DocWatcher will pick up InsertRemote events and trigger
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

    // =========================================================================
    // Two-phase import methods (for pipeline)
    // =========================================================================

    /// Phase 1: Store PDF source bytes into a collection.
    ///
    /// This is the first phase of the pipeline - just copies the PDF bytes
    /// to iroh storage without text extraction. Creates:
    /// - `files/{id}/meta` - metadata with no canonical extraction yet
    /// - `files/{id}/source` - original PDF bytes
    ///
    /// The document's id is the source's BLAKE3 hash.
    ///
    /// Returns the new document's metadata for the next phase.
    ///
    /// Note: No extraction exists until `extract_and_dispatch()` is called.
    pub async fn store_pdf_source(
        &self,
        path: &std::path::Path,
        namespace_id: NamespaceId,
    ) -> Result<DocumentMetadata> {
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown.pdf".to_string());

        let pdf_bytes = std::fs::read(path).context("Failed to read PDF file")?;

        let metadata = self
            .add_document(namespace_id, &file_name, "application/pdf", &pdf_bytes)
            .await?;

        tracing::info!(
            doc_id = %metadata.id,
            "Stored PDF source for {file_name}"
        );

        Ok(metadata)
    }

    /// Phase 2: Extract text from the stored PDF and write it out.
    ///
    /// Extracts per-page text via [`crate::pdf::extract_text_from_bytes`] and
    /// writes `files/{id}/extractions/{parser_key}`. The extraction becomes
    /// canonical only when the document has no canonical extraction yet.
    ///
    /// Returns `Ok(Some(metadata))` on success, `Ok(None)` when the
    /// document has been deleted (job queued before deletion), and
    /// `Err(...)` for actual failures.
    pub async fn extract_and_dispatch(
        &self,
        namespace_id: NamespaceId,
        doc_id: &str,
    ) -> Result<Option<DocumentMetadata>> {
        let source_bytes = match self.get_document_source(namespace_id, doc_id).await? {
            Some(b) => b,
            None => {
                tracing::warn!(
                    doc_id = %doc_id,
                    "Source not found; document was likely deleted — skipping extract"
                );
                return Ok(None);
            }
        };

        let mut metadata = match self.get_document(namespace_id, doc_id).await? {
            Some(metadata) => metadata,
            None => {
                tracing::warn!(
                    doc_id = %doc_id,
                    "Metadata not found; document was likely deleted — skipping extract"
                );
                return Ok(None);
            }
        };

        let extraction =
            tokio::task::spawn_blocking(move || crate::pdf::extract_text_from_bytes(source_bytes))
                .await
                .context("Extract task panicked")??;

        self.write_extraction(namespace_id, doc_id, &extraction)
            .await?;

        if metadata.extraction.is_none() {
            self.set_canonical_extraction(namespace_id, doc_id, &extraction.parser)
                .await?;
            metadata = self
                .get_document(namespace_id, doc_id)
                .await?
                .context("Document vanished during extraction")?;
        }

        tracing::info!(
            doc_id = %doc_id,
            page_count = extraction.page_count(),
            text_len = extraction.text.len(),
            "Extracted text for {}",
            metadata.name
        );

        Ok(Some(metadata))
    }

    /// Internal helper: serialize + store the metadata entry on an open
    /// doc handle.
    async fn store_meta_inner(
        &self,
        doc: &iroh_docs::api::Doc,
        doc_id: &str,
        metadata: &DocumentMetadata,
    ) -> Result<()> {
        let metadata_bytes = serde_json::to_vec(metadata)?;
        let meta_hash = self.store_blob(&metadata_bytes).await?;
        let meta_key = doc_meta_key(doc_id);
        doc.set_hash(
            self.author_id,
            meta_key.into_bytes(),
            meta_hash,
            metadata_bytes.len() as u64,
        )
        .await?;
        Ok(())
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

    /// Add a document, returning its id: the source's BLAKE3 hash.
    async fn add_document(
        storage: &Storage,
        collection_id: NamespaceId,
        name: &str,
        source: &[u8],
    ) -> String {
        storage
            .add_document(collection_id, name, "application/pdf", source)
            .await
            .unwrap()
            .id
    }

    fn parser(name: &str) -> ParserId {
        ParserId {
            name: name.to_string(),
            version: "1".to_string(),
        }
    }

    fn extraction(parser: ParserId, text: &str, page_ends: Vec<usize>) -> Extraction {
        let mut start = 0;
        let units = page_ends
            .into_iter()
            .enumerate()
            .map(|(index, end)| {
                let unit = Unit {
                    number: index + 1,
                    spans: vec![crate::text::TextSpan { start, end }],
                };
                start = end;
                unit
            })
            .collect();
        Extraction {
            parser,
            text: text.to_string(),
            unit_kind: Some(UnitKind::Page),
            units,
        }
    }

    #[tokio::test]
    async fn test_add_and_list_documents() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp_dir.path()).await.unwrap();

        let (collection_id, _) = storage.create_collection("My Docs").await.unwrap();

        let source_content = b"PDF bytes here";
        let doc_id = add_document(&storage, collection_id, "test.pdf", source_content).await;

        let extraction = extraction(parser("test-parser"), "0123456789", vec![2, 4, 6, 8, 10]);
        storage
            .write_extraction(collection_id, &doc_id, &extraction)
            .await
            .unwrap();
        storage
            .set_canonical_extraction(collection_id, &doc_id, &extraction.parser)
            .await
            .unwrap();

        // Count should be 1 (counts documents, not entries)
        let count = storage.count_documents(collection_id).await.unwrap();
        assert_eq!(count, 1);

        // List documents (should return 1 document metadata)
        let docs = storage.list_documents(collection_id).await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].id, doc_id);
        assert_eq!(docs[0].name, "test.pdf");
        assert_eq!(docs[0].extraction, Some(extraction.parser.clone()));
        assert_eq!(
            storage.page_count(collection_id, &doc_id).await.unwrap(),
            Some(5)
        );

        // Verify we can retrieve the canonical extraction and source
        let stored = storage
            .get_canonical_extraction(collection_id, &doc_id)
            .await
            .unwrap();
        assert_eq!(stored, Some(extraction));

        let source = storage
            .get_document_source(collection_id, &doc_id)
            .await
            .unwrap();
        assert_eq!(source, Some(source_content.to_vec()));
    }

    #[tokio::test]
    async fn test_multiple_extractions_and_canonical_selection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp_dir.path()).await.unwrap();
        let (collection_id, _) = storage.create_collection("My Docs").await.unwrap();

        let doc_id = add_document(&storage, collection_id, "test.pdf", b"source").await;

        let first = extraction(parser("first"), "one page", vec![8]);
        let second = extraction(parser("second"), "two pages", vec![3, 9]);
        storage
            .write_extraction(collection_id, &doc_id, &first)
            .await
            .unwrap();
        storage
            .write_extraction(collection_id, &doc_id, &second)
            .await
            .unwrap();

        // Writing extractions does not choose a canonical one.
        assert_eq!(
            storage
                .get_canonical_extraction(collection_id, &doc_id)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            storage
                .get_extraction(collection_id, &doc_id, &first.parser)
                .await
                .unwrap(),
            Some(first.clone())
        );
        assert_eq!(
            storage
                .get_extraction(collection_id, &doc_id, &second.parser)
                .await
                .unwrap(),
            Some(second.clone())
        );

        storage
            .set_canonical_extraction(collection_id, &doc_id, &first.parser)
            .await
            .unwrap();
        assert_eq!(
            storage
                .get_canonical_extraction(collection_id, &doc_id)
                .await
                .unwrap(),
            Some(first.clone())
        );
        assert_eq!(
            storage.page_count(collection_id, &doc_id).await.unwrap(),
            Some(1)
        );

        storage
            .set_canonical_extraction(collection_id, &doc_id, &second.parser)
            .await
            .unwrap();
        assert_eq!(
            storage
                .get_canonical_extraction(collection_id, &doc_id)
                .await
                .unwrap(),
            Some(second)
        );
        assert_eq!(
            storage.page_count(collection_id, &doc_id).await.unwrap(),
            Some(2)
        );

        // Selecting an extraction that was never written throws.
        assert!(storage
            .set_canonical_extraction(collection_id, &doc_id, &parser("missing"))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn test_delete_document_removes_all_document_entries() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp_dir.path()).await.unwrap();

        let (collection_id, _) = storage.create_collection("My Docs").await.unwrap();

        let source = b"source1";
        let doc_id = add_document(&storage, collection_id, "first.pdf", source).await;
        add_document(&storage, collection_id, "second.pdf", b"source2").await;

        let worked = extraction(parser("worked"), "doc one", vec![7]);
        storage
            .write_extraction(collection_id, &doc_id, &worked)
            .await
            .unwrap();
        storage
            .set_canonical_extraction(collection_id, &doc_id, &worked.parser)
            .await
            .unwrap();

        storage
            .delete_document(collection_id, &doc_id)
            .await
            .unwrap();

        // Every entry under the document prefix is gone: metadata, source
        // and extractions.
        assert!(storage
            .get_document(collection_id, &doc_id)
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            storage
                .get_document_source(collection_id, &doc_id)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            storage
                .get_extraction(collection_id, &doc_id, &worked.parser)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            storage
                .get_canonical_extraction(collection_id, &doc_id)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            storage.page_count(collection_id, &doc_id).await.unwrap(),
            None
        );

        // The document disappears from the collection's listing.
        let docs = storage.list_documents(collection_id).await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].name, "second.pdf");
        assert_eq!(storage.count_documents(collection_id).await.unwrap(), 1);

        // Its source can be imported again as a fresh document, without the
        // deleted document's extractions.
        let re_added = add_document(&storage, collection_id, "first.pdf", source).await;
        assert_eq!(re_added, doc_id);
        assert_eq!(
            storage
                .get_extraction(collection_id, &doc_id, &worked.parser)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            storage.page_count(collection_id, &doc_id).await.unwrap(),
            None
        );
        assert_eq!(storage.count_documents(collection_id).await.unwrap(), 2);
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
    async fn test_duplicate_detection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp_dir.path()).await.unwrap();

        let (collection_id, _) = storage.create_collection("Duplicates Test").await.unwrap();

        let source_content = b"unique source content";
        let doc_id = add_document(&storage, collection_id, "report.pdf", source_content).await;

        // The id is the hash iroh computed for the source, so the same source
        // imported again is the same document.
        assert_eq!(doc_id, Hash::new(source_content).to_hex());
        assert!(storage
            .add_document(collection_id, "copy.pdf", "application/pdf", source_content)
            .await
            .is_err());
        assert_eq!(storage.count_documents(collection_id).await.unwrap(), 1);

        // A different source is a different document.
        add_document(&storage, collection_id, "other.pdf", b"different content").await;
        assert_eq!(storage.count_documents(collection_id).await.unwrap(), 2);

        // Deleting and re-importing the source yields one document again.
        storage
            .delete_document(collection_id, &doc_id)
            .await
            .unwrap();
        assert_eq!(storage.count_documents(collection_id).await.unwrap(), 1);
        let re_added = add_document(&storage, collection_id, "report.pdf", source_content).await;
        assert_eq!(re_added, doc_id);
        assert_eq!(storage.count_documents(collection_id).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn test_two_phase_import() {
        /// One-page PDF with a standard Helvetica text run.
        const TEXT_PDF: &[u8] = include_bytes!("../../tests/fixtures/text.pdf");

        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp_dir.path()).await.unwrap();
        let (collection_id, _) = storage.create_collection("Two Phase Test").await.unwrap();

        // Create a test PDF file with extractable digital text.
        let pdf_path = temp_dir.path().join("test.pdf");
        let pdf_bytes = TEXT_PDF.to_vec();
        std::fs::write(&pdf_path, &pdf_bytes).unwrap();

        // Phase 1: Store source
        let document = storage
            .store_pdf_source(&pdf_path, collection_id)
            .await
            .unwrap();
        let doc_id = document.id;
        assert_eq!(doc_id, Hash::new(&pdf_bytes).to_hex());

        // After phase 1: metadata exists but no extraction is canonical.
        let meta1 = storage.get_document(collection_id, &doc_id).await.unwrap();
        assert!(meta1.is_some());
        let meta1 = meta1.unwrap();
        assert_eq!(meta1.extraction, None);
        assert_eq!(
            storage.page_count(collection_id, &doc_id).await.unwrap(),
            None
        );
        assert_eq!(
            storage
                .get_canonical_extraction(collection_id, &doc_id)
                .await
                .unwrap(),
            None
        );

        // Source should exist
        let source = storage
            .get_document_source(collection_id, &doc_id)
            .await
            .unwrap();
        assert!(source.is_some());
        assert_eq!(source.unwrap(), pdf_bytes);

        // Phase 2: Extract and store text
        let extracted = storage
            .extract_and_dispatch(collection_id, &doc_id)
            .await
            .unwrap()
            .expect("document should exist");
        assert_eq!(extracted.extraction, Some(parser("mupdf-pdf-text")));

        // The canonical extraction carries the page structure and text.
        assert_eq!(
            storage.page_count(collection_id, &doc_id).await.unwrap(),
            Some(1)
        );
        let canonical = storage
            .get_canonical_extraction(collection_id, &doc_id)
            .await
            .unwrap()
            .expect("canonical extraction should exist");
        assert_eq!(canonical.unit_kind, Some(UnitKind::Page));
        assert_eq!(canonical.units.len(), 1);
        assert!(!canonical.text.is_empty());

        // The same PDF is the same document, so it cannot be imported twice.
        assert!(storage
            .store_pdf_source(&pdf_path, collection_id)
            .await
            .is_err());
        assert_eq!(storage.count_documents(collection_id).await.unwrap(), 1);
    }
}
