use super::*;
use crate::core::{search, AppState, Config, Storage};
use milli::update::IndexerConfig;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::test::MockRuntime;
use tauri::Manager;
use tokio::sync::{Mutex, RwLock};

/// Create a test AppState with temporary directories
async fn create_test_state(temp_dir: &std::path::Path) -> AppState {
    let config = Config {
        data_dir: temp_dir.to_path_buf(),
        iroh_dir: temp_dir.join("iroh"),
        search_dir: temp_dir.join("search"),
        models_dir: temp_dir.join("models"),
    };
    config.ensure_dirs().unwrap();

    let storage = Storage::open(&config.iroh_dir).await.unwrap();
    let index = search::open_index(&config.search_dir).unwrap();
    let indexer_config = IndexerConfig::default();

    AppState {
        config,
        storage: Arc::new(RwLock::new(Some(storage))),
        search: Arc::new(RwLock::new(Some(index))),
        indexer_config: Arc::new(Mutex::new(indexer_config)),
        agent_model: Arc::new(RwLock::new(None)),
        conversations: Arc::new(RwLock::new(HashMap::new())),
        active_generations: Arc::new(RwLock::new(HashMap::new())),
    }
}

/// Helper to create a mock Tauri app for testing commands
fn create_test_app(state: AppState) -> tauri::App<MockRuntime> {
    tauri::test::mock_builder()
        .manage(state)
        .build(tauri::generate_context!())
        .unwrap()
}

// ============================================================================
// Collection Command Tests
// ============================================================================

#[tokio::test]
async fn test_get_collections_empty() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state = create_test_state(temp_dir.path()).await;
    let app = create_test_app(state);

    let state = app.state::<AppState>();
    let collections = get_collections(state).await.unwrap();

    assert!(collections.is_empty());
}

#[tokio::test]
async fn test_create_collection() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state = create_test_state(temp_dir.path()).await;
    let app = create_test_app(state);

    let state = app.state::<AppState>();
    let collection = create_collection("Test Collection".to_string(), state.clone())
        .await
        .unwrap();

    assert_eq!(collection.name, "Test Collection");
    assert_eq!(collection.document_count, 0);
    assert!(!collection.id.is_empty());
    assert!(!collection.created_at.is_empty());

    // Verify it shows up in list
    let collections = get_collections(state).await.unwrap();
    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].name, "Test Collection");
}

#[tokio::test]
async fn test_create_multiple_collections() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state = create_test_state(temp_dir.path()).await;
    let app = create_test_app(state);

    let state = app.state::<AppState>();

    let col1 = create_collection("First".to_string(), state.clone())
        .await
        .unwrap();
    let col2 = create_collection("Second".to_string(), state.clone())
        .await
        .unwrap();

    assert_ne!(col1.id, col2.id);

    let collections = get_collections(state).await.unwrap();
    assert_eq!(collections.len(), 2);
}

#[tokio::test]
async fn test_delete_collection() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state = create_test_state(temp_dir.path()).await;
    let app = create_test_app(state);

    let state = app.state::<AppState>();

    // Create a collection
    let collection = create_collection("To Delete".to_string(), state.clone())
        .await
        .unwrap();

    // Delete it
    delete_collection(collection.id.clone(), state.clone())
        .await
        .unwrap();

    // Verify it's gone
    let collections = get_collections(state).await.unwrap();
    assert!(collections.is_empty());
}

#[tokio::test]
async fn test_delete_collection_invalid_id() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state = create_test_state(temp_dir.path()).await;
    let app = create_test_app(state);

    let state = app.state::<AppState>();

    let result = delete_collection("invalid-id".to_string(), state).await;

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Invalid collection ID"));
}

// ============================================================================
// Document Command Tests
// ============================================================================

#[tokio::test]
async fn test_get_documents_empty() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state = create_test_state(temp_dir.path()).await;
    let app = create_test_app(state);

    let state = app.state::<AppState>();

    let collection = create_collection("Empty".to_string(), state.clone())
        .await
        .unwrap();

    let documents = get_documents(collection.id, state).await.unwrap();
    assert!(documents.is_empty());
}

#[tokio::test]
async fn test_get_documents_invalid_collection() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state = create_test_state(temp_dir.path()).await;
    let app = create_test_app(state);

    let state = app.state::<AppState>();

    let result = get_documents("invalid-id".to_string(), state).await;

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Invalid collection ID"));
}

// ============================================================================
// Import PDF Command Tests
// ============================================================================

#[tokio::test]
async fn test_import_pdf_invalid_collection() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state = create_test_state(temp_dir.path()).await;
    let app = create_test_app(state);

    let state = app.state::<AppState>();

    let result = import_pdf(
        "/some/path.pdf".to_string(),
        "invalid-id".to_string(),
        state,
    )
    .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Invalid collection ID"));
}

#[tokio::test]
async fn test_import_pdf_file_not_found() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state = create_test_state(temp_dir.path()).await;
    let app = create_test_app(state);

    let state = app.state::<AppState>();

    let collection = create_collection("Test".to_string(), state.clone())
        .await
        .unwrap();

    let result = import_pdf(
        "/nonexistent/file.pdf".to_string(),
        collection.id,
        state,
    )
    .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Failed to read PDF file"));
}

#[tokio::test]
async fn test_import_pdf_success() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Create a test PDF
    let pdf_path = temp_dir.path().join("test.pdf");
    let pdf_bytes = create_test_pdf("Hello from the test PDF");
    std::fs::write(&pdf_path, &pdf_bytes).unwrap();

    let state = create_test_state(temp_dir.path()).await;
    let app = create_test_app(state);

    let state = app.state::<AppState>();

    let collection = create_collection("Test".to_string(), state.clone())
        .await
        .unwrap();

    let doc = import_pdf(
        pdf_path.to_string_lossy().to_string(),
        collection.id.clone(),
        state.clone(),
    )
    .await
    .unwrap();

    assert_eq!(doc.name, "test.pdf");
    assert_eq!(doc.page_count, 1);
    assert!(!doc.id.is_empty());
    assert!(!doc.pdf_hash.is_empty());
    assert!(!doc.text_hash.is_empty());

    // Verify document shows up in collection
    let documents = get_documents(collection.id.clone(), state.clone())
        .await
        .unwrap();
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].id, doc.id);

    // Verify collection count updated
    let collections = get_collections(state).await.unwrap();
    assert_eq!(collections[0].document_count, 1);
}

#[tokio::test]
async fn test_delete_document() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Create test PDFs
    let pdf1_path = temp_dir.path().join("doc1.pdf");
    let pdf2_path = temp_dir.path().join("doc2.pdf");
    std::fs::write(&pdf1_path, create_test_pdf("First document")).unwrap();
    std::fs::write(&pdf2_path, create_test_pdf("Second document")).unwrap();

    let state = create_test_state(temp_dir.path()).await;
    let app = create_test_app(state);

    let state = app.state::<AppState>();

    let collection = create_collection("Test".to_string(), state.clone())
        .await
        .unwrap();

    let doc1 = import_pdf(
        pdf1_path.to_string_lossy().to_string(),
        collection.id.clone(),
        state.clone(),
    )
    .await
    .unwrap();

    let _doc2 = import_pdf(
        pdf2_path.to_string_lossy().to_string(),
        collection.id.clone(),
        state.clone(),
    )
    .await
    .unwrap();

    // Delete first document
    delete_document(collection.id.clone(), doc1.id.clone(), state.clone())
        .await
        .unwrap();

    // Small delay for background task
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Verify only one document remains
    let documents = get_documents(collection.id, state).await.unwrap();
    assert_eq!(documents.len(), 1);
    assert_ne!(documents[0].id, doc1.id);
}

// ============================================================================
// Search Command Tests
// ============================================================================

#[tokio::test]
async fn test_search_empty_index() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state = create_test_state(temp_dir.path()).await;
    let app = create_test_app(state);

    let state = app.state::<AppState>();

    let result = search("test query".to_string(), None, None, None, state)
        .await
        .unwrap();

    assert!(result.hits.is_empty());
    assert_eq!(result.total_hits, 0);
    assert_eq!(result.page, 0);
    assert_eq!(result.page_size, 20);
}

#[tokio::test]
async fn test_search_finds_document() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Create a test PDF with searchable content
    let pdf_path = temp_dir.path().join("climate.pdf");
    std::fs::write(&pdf_path, create_test_pdf("Climate change research paper")).unwrap();

    let state = create_test_state(temp_dir.path()).await;
    let app = create_test_app(state);

    let state = app.state::<AppState>();

    let collection = create_collection("Research".to_string(), state.clone())
        .await
        .unwrap();

    import_pdf(
        pdf_path.to_string_lossy().to_string(),
        collection.id.clone(),
        state.clone(),
    )
    .await
    .unwrap();

    // Search for content
    let result = search("climate".to_string(), None, None, None, state)
        .await
        .unwrap();

    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.total_hits, 1);
    assert_eq!(result.hits[0].document.name, "climate.pdf");
}

#[tokio::test]
async fn test_search_with_collection_filter() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Create test PDFs
    let pdf1_path = temp_dir.path().join("doc1.pdf");
    let pdf2_path = temp_dir.path().join("doc2.pdf");
    std::fs::write(&pdf1_path, create_test_pdf("Important research document")).unwrap();
    std::fs::write(&pdf2_path, create_test_pdf("Another research paper")).unwrap();

    let state = create_test_state(temp_dir.path()).await;
    let app = create_test_app(state);

    let state = app.state::<AppState>();

    let col1 = create_collection("Collection A".to_string(), state.clone())
        .await
        .unwrap();
    let col2 = create_collection("Collection B".to_string(), state.clone())
        .await
        .unwrap();

    import_pdf(
        pdf1_path.to_string_lossy().to_string(),
        col1.id.clone(),
        state.clone(),
    )
    .await
    .unwrap();

    import_pdf(
        pdf2_path.to_string_lossy().to_string(),
        col2.id.clone(),
        state.clone(),
    )
    .await
    .unwrap();

    // Search all - should find both
    let all = search("research".to_string(), None, None, None, state.clone())
        .await
        .unwrap();
    assert_eq!(all.hits.len(), 2);

    // Search with filter - should find only one
    let filtered = search(
        "research".to_string(),
        None,
        None,
        Some(vec![col1.id.clone()]),
        state,
    )
    .await
    .unwrap();
    assert_eq!(filtered.hits.len(), 1);
    assert_eq!(filtered.hits[0].collection_id, col1.id);
}

#[tokio::test]
async fn test_search_pagination() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state = create_test_state(temp_dir.path()).await;
    let app = create_test_app(state);

    let state = app.state::<AppState>();

    let collection = create_collection("Test".to_string(), state.clone())
        .await
        .unwrap();

    // Create and import multiple PDFs
    for i in 0..5 {
        let pdf_path = temp_dir.path().join(format!("doc{}.pdf", i));
        std::fs::write(&pdf_path, create_test_pdf(&format!("Document number {}", i))).unwrap();
        import_pdf(
            pdf_path.to_string_lossy().to_string(),
            collection.id.clone(),
            state.clone(),
        )
        .await
        .unwrap();
    }

    // Search with page_size = 2
    let page0 = search(
        "document".to_string(),
        Some(0),
        Some(2),
        None,
        state.clone(),
    )
    .await
    .unwrap();

    assert_eq!(page0.hits.len(), 2);
    assert_eq!(page0.page, 0);
    assert_eq!(page0.page_size, 2);
    assert_eq!(page0.total_hits, 5);

    // Get second page
    let page1 = search(
        "document".to_string(),
        Some(1),
        Some(2),
        None,
        state,
    )
    .await
    .unwrap();

    assert_eq!(page1.hits.len(), 2);
    assert_eq!(page1.page, 1);
}

// ============================================================================
// Model Command Tests
// ============================================================================

#[tokio::test]
async fn test_get_available_models() {
    let models = get_available_models().await.unwrap();

    assert!(!models.is_empty());
    // Check that each model has required fields
    for model in &models {
        assert!(!model.id.is_empty());
        assert!(!model.name.is_empty());
    }
}

#[tokio::test]
async fn test_get_model_status_not_downloaded() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state = create_test_state(temp_dir.path()).await;
    let app = create_test_app(state);

    let state = app.state::<AppState>();

    let status = get_model_status(None, state).await.unwrap();

    match status {
        ModelStatus::NotDownloaded => {}
        _ => panic!("Expected NotDownloaded status for fresh install"),
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Create a minimal PDF with the given text content (same as in pdf/extractor tests)
fn create_test_pdf(text: &str) -> Vec<u8> {
    use lopdf::{dictionary, Document, Object, Stream};

    let mut doc = Document::with_version("1.4");

    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });

    let content = format!(
        "BT /F1 12 Tf 100 700 Td ({}) Tj ET",
        text.replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)")
    );
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));

    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! {
            "F1" => font_id,
        },
    });

    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => resources_id,
        "Contents" => content_id,
    });

    let pages_id = doc.add_object(dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
    });

    if let Ok(page) = doc.get_object_mut(page_id) {
        if let Object::Dictionary(ref mut dict) = page {
            dict.set("Parent", pages_id);
        }
    }

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });

    doc.trailer.set("Root", catalog_id);

    let mut buffer = Vec::new();
    doc.save_to(&mut buffer).unwrap();
    buffer
}
