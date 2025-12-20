use milli::update::IndexerConfig;

use crate::core::{
    config::Config,
    search::{get_document_count, index_documents_batch, open_index, DocToIndex},
    storage::Storage,
};

/// Rebuild the search index from all stored documents
pub fn index_rebuild() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("insight=info".parse().unwrap()),
        )
        .init();

    tracing::info!("Rebuilding search index...");

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    if let Err(e) = rt.block_on(do_index_rebuild()) {
        tracing::error!("Index rebuild failed: {}", e);
        std::process::exit(1);
    }

    tracing::info!("Index rebuild complete");
}

async fn do_index_rebuild() -> anyhow::Result<()> {
    let config = Config::load_or_default();

    // Open storage to read documents
    let mut storage = Storage::open(&config.iroh_dir).await?;

    // Delete existing index and recreate
    if config.search_dir.exists() {
        tracing::info!("Removing existing index at {:?}", config.search_dir);
        std::fs::remove_dir_all(&config.search_dir)?;
    }

    let index = open_index(&config.search_dir)?;

    // Create a single IndexerConfig for all indexing operations
    let indexer_config = IndexerConfig::default();

    // Get all collections
    let collections = storage.list_collections().await?;
    tracing::info!("Found {} collections", collections.len());

    let mut total_docs = 0;
    let mut docs_to_index = Vec::new();

    for (namespace_id, collection_meta) in &collections {
        tracing::info!(
            "Processing collection '{}' ({})",
            collection_meta.name,
            namespace_id
        );

        let documents = storage.list_documents(*namespace_id).await?;
        total_docs += documents.len();

        for doc in documents {
            // Fetch text content from blob storage
            let text_hash: iroh_blobs::Hash = doc.text_hash.parse().map_err(|_| {
                anyhow::anyhow!(
                    "Invalid text hash for document {}: {}",
                    doc.id,
                    doc.text_hash
                )
            })?;

            match storage.get_blob(&text_hash).await? {
                Some(text_bytes) => {
                    let text = String::from_utf8_lossy(&text_bytes).to_string();
                    let collection_id = namespace_id.to_string();

                    docs_to_index.push(DocToIndex {
                        id: doc.id.clone(),
                        name: doc.name.clone(),
                        content: text,
                        collection_id,
                    });
                    tracing::debug!("Prepared document '{}' ({})", doc.name, doc.id);
                }
                None => {
                    tracing::warn!(
                        "Text blob not found for document '{}' (hash: {})",
                        doc.name,
                        doc.text_hash
                    );
                }
            }
        }
    }

    // Batch index all documents at once
    let indexed_count = docs_to_index.len();
    if !docs_to_index.is_empty() {
        tracing::info!("Batch indexing {} documents...", indexed_count);
        index_documents_batch(&index, &indexer_config, docs_to_index)?;
    }

    tracing::info!(
        "Indexed {}/{} documents from {} collections",
        indexed_count,
        total_docs,
        collections.len()
    );

    Ok(())
}

/// Show status of storage vs search index
pub fn index_status() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("insight=info".parse().unwrap()),
        )
        .init();

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    if let Err(e) = rt.block_on(do_index_status()) {
        tracing::error!("Status check failed: {}", e);
        std::process::exit(1);
    }
}

async fn do_index_status() -> anyhow::Result<()> {
    let config = Config::load_or_default();

    // Check if paths exist
    println!("Data directories:");
    println!(
        "  iroh:   {:?} (exists: {})",
        config.iroh_dir,
        config.iroh_dir.exists()
    );
    println!(
        "  search: {:?} (exists: {})",
        config.search_dir,
        config.search_dir.exists()
    );
    println!();

    // Open storage to count documents
    let mut storage = Storage::open(&config.iroh_dir).await?;
    let collections = storage.list_collections().await?;

    let mut storage_doc_count = 0;
    println!("Collections in storage: {}", collections.len());
    for (namespace_id, collection_meta) in &collections {
        let documents = storage.list_documents(*namespace_id).await?;
        println!(
            "  - '{}' ({}) : {} documents",
            collection_meta.name,
            namespace_id,
            documents.len()
        );
        storage_doc_count += documents.len();
    }
    println!("Total documents in storage: {}", storage_doc_count);
    println!();

    // Open search index to count documents
    if config.search_dir.exists() {
        let index = open_index(&config.search_dir)?;
        let index_doc_count = get_document_count(&index)?;
        println!("Documents in search index: {}", index_doc_count);
        println!();

        if storage_doc_count as u64 != index_doc_count {
            println!(
                "⚠️  MISMATCH: Storage has {} documents, index has {}",
                storage_doc_count, index_doc_count
            );
            println!("   Run 'insight index rebuild' to fix this.");
        } else {
            println!("✓ Storage and index are in sync");
        }
    } else {
        println!("Search index does not exist yet.");
        if storage_doc_count > 0 {
            println!(
                "⚠️  {} documents in storage are not indexed!",
                storage_doc_count
            );
            println!("   Run 'insight index rebuild' to create the index.");
        }
    }

    Ok(())
}
