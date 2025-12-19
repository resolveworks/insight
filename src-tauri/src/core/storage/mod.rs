use std::path::Path;

use anyhow::{Context, Result};
use iroh_blobs::store::fs::Store as BlobStore;
use iroh_blobs::store::{Map, MapEntry, Store as BlobStoreTrait};
use iroh_blobs::{BlobFormat, Hash, HashAndFormat};
use iroh_docs::store::fs::Store as DocStore;
use iroh_docs::{Author, NamespaceId, NamespaceSecret};
use iroh_io::AsyncSliceReader;
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
}

/// Storage layer using iroh for P2P content-addressed storage
pub struct Storage {
    /// Content-addressed blob storage (PDFs, extracted text)
    pub blobs: BlobStore,
    /// CRDT document store (metadata, collections)
    pub docs: DocStore,
    /// Local author for writing entries
    pub author: Author,
}

impl Storage {
    /// Initialize storage at the given path
    pub async fn open(path: &Path) -> Result<Self> {
        std::fs::create_dir_all(path)?;

        let blobs_path = path.join("blobs");
        let docs_path = path.join("docs.redb");

        let blobs = BlobStore::load(&blobs_path)
            .await
            .context("Failed to open blob store")?;

        let mut docs = DocStore::persistent(&docs_path)
            .context("Failed to open docs store")?;

        // Get or create a local author for this node
        let author = {
            let mut authors = docs.list_authors()?;
            if let Some(existing) = authors.next() {
                existing?
            } else {
                docs.new_author(&mut rand::thread_rng())?
            }
        };

        tracing::info!(
            "Storage opened at {:?}, author: {}",
            path,
            author.id().fmt_short()
        );

        Ok(Self { blobs, docs, author })
    }

    /// Store bytes in blob storage and return the hash
    /// Creates a permanent tag to prevent garbage collection
    pub async fn store_blob(&self, data: &[u8]) -> Result<Hash> {
        let bytes = bytes::Bytes::copy_from_slice(data);
        let temp_tag = self.blobs.import_bytes(bytes, BlobFormat::Raw).await?;
        let hash = *temp_tag.hash();

        // Create a permanent tag so the blob isn't garbage collected
        // Use the hash hex as the tag name
        let hash_and_format = HashAndFormat::raw(hash);
        let tag_name = iroh_blobs::Tag::from(hash.to_string());
        self.blobs.set_tag(tag_name, hash_and_format).await?;

        Ok(hash)
    }

    /// Get bytes from blob storage by hash
    pub async fn get_blob(&self, hash: &Hash) -> Result<Option<Vec<u8>>> {
        let entry = match self.blobs.get(hash).await? {
            Some(e) => e,
            None => return Ok(None),
        };

        if !entry.is_complete() {
            return Ok(None);
        }

        let size = entry.size().value();
        // Use inherent data_reader method which returns DataReader directly
        let mut reader = entry.data_reader();
        let data = reader.read_at(0, size as usize).await?;

        Ok(Some(data.to_vec()))
    }

    /// Create a new collection (namespace) with the given name
    pub async fn create_collection(
        &mut self,
        name: &str,
    ) -> Result<(NamespaceId, CollectionMetadata)> {
        let metadata = CollectionMetadata {
            name: name.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        // Store metadata in blob storage first
        let metadata_bytes = serde_json::to_vec(&metadata)?;
        let hash = self.store_blob(&metadata_bytes).await?;
        let len = metadata_bytes.len() as u64;

        // Now create the namespace and store the reference
        let namespace_secret = NamespaceSecret::new(&mut rand::thread_rng());
        let mut replica = self.docs.new_replica(namespace_secret)?;

        // Store reference in iroh-docs under `_collection` key
        replica.insert(b"_collection", &self.author, hash, len)?;

        let namespace_id = replica.id();
        self.docs.close_replica(namespace_id);

        // Flush to ensure data is persisted
        self.docs.flush()?;

        tracing::info!("Created collection '{}' with id {}", name, namespace_id);

        Ok((namespace_id, metadata))
    }

    /// List all collections with their metadata
    pub async fn list_collections(&mut self) -> Result<Vec<(NamespaceId, CollectionMetadata)>> {
        let mut collections = Vec::new();

        // Get all namespaces
        let namespaces: Vec<_> = self.docs.list_namespaces()?.collect::<Result<_>>()?;
        tracing::debug!("Found {} namespaces", namespaces.len());

        for (namespace_id, kind) in namespaces {
            tracing::debug!("Checking namespace {} ({:?})", namespace_id, kind);
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
        &mut self,
        namespace_id: NamespaceId,
    ) -> Result<Option<CollectionMetadata>> {
        // Query for the _collection entry
        let query = iroh_docs::store::Query::key_exact(b"_collection").build();
        let mut entries = self.docs.get_many(namespace_id, query)?;

        let metadata = if let Some(entry) = entries.next() {
            let entry = entry?;
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

        Ok(metadata)
    }

    /// Count documents in a collection
    pub fn count_documents(&mut self, namespace_id: NamespaceId) -> Result<usize> {
        // Query for all entries with prefix "files/"
        let query = iroh_docs::store::Query::key_prefix(b"files/").build();
        let entries = self.docs.get_many(namespace_id, query)?;
        Ok(entries.count())
    }

    /// Add a document to a collection
    pub async fn add_document(
        &mut self,
        namespace_id: NamespaceId,
        metadata: DocumentMetadata,
    ) -> Result<()> {
        // Store metadata in blob storage
        let metadata_bytes = serde_json::to_vec(&metadata)?;
        let hash = self.store_blob(&metadata_bytes).await?;
        let len = metadata_bytes.len() as u64;

        // Store reference in iroh-docs under `files/{id}` key
        let key = format!("files/{}", metadata.id);
        let mut replica = self.docs.open_replica(&namespace_id)?;
        replica.insert(key.as_bytes(), &self.author, hash, len)?;
        self.docs.close_replica(namespace_id);
        self.docs.flush()?;

        tracing::info!(
            "Added document '{}' to collection {}",
            metadata.name,
            namespace_id
        );

        Ok(())
    }

    /// List all documents in a collection
    pub async fn list_documents(
        &mut self,
        namespace_id: NamespaceId,
    ) -> Result<Vec<DocumentMetadata>> {
        let query = iroh_docs::store::Query::key_prefix(b"files/").build();
        let entries = self.docs.get_many(namespace_id, query)?;

        let mut documents = Vec::new();
        for entry in entries {
            let entry = entry?;
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

        Ok(documents)
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
        let mut storage = Storage::open(temp_dir.path()).await.unwrap();

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
        let mut storage = Storage::open(temp_dir.path()).await.unwrap();

        let (id, _) = storage.create_collection("My Docs").await.unwrap();

        let metadata = storage.get_collection_metadata(id).await.unwrap();
        assert!(metadata.is_some());
        assert_eq!(metadata.unwrap().name, "My Docs");
    }

    #[tokio::test]
    async fn test_count_documents_empty() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::open(temp_dir.path()).await.unwrap();

        let (id, _) = storage.create_collection("Empty").await.unwrap();
        let count = storage.count_documents(id).unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_add_and_list_documents() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::open(temp_dir.path()).await.unwrap();

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
        };
        storage.add_document(collection_id, doc).await.unwrap();

        // Count should be 1
        let count = storage.count_documents(collection_id).unwrap();
        assert_eq!(count, 1);

        // List documents
        let docs = storage.list_documents(collection_id).await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].id, "doc-1");
        assert_eq!(docs[0].name, "test.pdf");
        assert_eq!(docs[0].page_count, 5);
    }
}
