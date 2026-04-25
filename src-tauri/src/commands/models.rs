use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::core::{models, search, AppState, ModelType};
use crate::error::{CommandError, CommandResult, ResultExt};

/// Model info for frontend (unified across types)
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub size_gb: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<usize>,
}

impl From<models::LanguageModelInfo> for ModelInfo {
    fn from(m: models::LanguageModelInfo) -> Self {
        Self {
            id: m.id,
            name: m.name,
            description: m.description,
            size_gb: m.size_gb,
            dimensions: None,
        }
    }
}

impl From<models::EmbeddingModelInfo> for ModelInfo {
    fn from(m: models::EmbeddingModelInfo) -> Self {
        Self {
            id: m.id,
            name: m.name,
            description: m.description,
            size_gb: m.size_gb,
            dimensions: Some(m.dimensions),
        }
    }
}

impl From<models::OcrModelInfo> for ModelInfo {
    fn from(m: models::OcrModelInfo) -> Self {
        Self {
            id: m.id,
            name: m.name,
            description: m.description,
            size_gb: m.size_gb,
            dimensions: None,
        }
    }
}

/// Get list of available models for a type
#[tauri::command]
pub async fn get_available_models(model_type: ModelType) -> CommandResult<Vec<ModelInfo>> {
    Ok(match model_type {
        ModelType::Language => models::available_language_models()
            .into_iter()
            .map(ModelInfo::from)
            .collect(),
        ModelType::Embedding => models::available_embedding_models()
            .into_iter()
            .map(ModelInfo::from)
            .collect(),
        ModelType::Ocr => models::available_ocr_models()
            .into_iter()
            .map(ModelInfo::from)
            .collect(),
    })
}

/// Model download status
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status")]
pub enum DownloadStatus {
    NotDownloaded,
    Ready,
}

/// Get download status for a model
#[tauri::command]
pub async fn get_model_status(
    model_type: ModelType,
    model_id: String,
    state: State<'_, AppState>,
) -> CommandResult<DownloadStatus> {
    let is_downloaded = match model_type {
        ModelType::Language => {
            let model = models::get_language_model(&model_id)
                .ok_or(CommandError::model_not_found(&model_id))?;
            state.model_downloader.is_downloaded(&model)
        }
        ModelType::Embedding => {
            let model = models::get_embedding_model(&model_id)
                .ok_or(CommandError::model_not_found(&model_id))?;
            state.model_downloader.is_downloaded(&model)
        }
        ModelType::Ocr => {
            let model =
                models::get_ocr_model(&model_id).ok_or(CommandError::model_not_found(&model_id))?;
            state.model_downloader.is_downloaded(&model)
        }
    };

    Ok(if is_downloaded {
        DownloadStatus::Ready
    } else {
        DownloadStatus::NotDownloaded
    })
}

/// Download a model with progress events
#[tauri::command]
pub async fn download_model(
    model_type: ModelType,
    model_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    use crate::core::{ModelDownloadProgress, ModelStatus};

    let (status_tx, mut status_rx) = tokio::sync::mpsc::channel::<ModelStatus>(10);
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<ModelDownloadProgress>(100);

    let app_status = app.clone();
    tokio::spawn(async move {
        while let Some(status) = status_rx.recv().await {
            let _ = app_status.emit("model-status-changed", &status);
        }
    });

    let app_progress = app.clone();
    tokio::spawn(async move {
        while let Some(progress) = progress_rx.recv().await {
            let _ = app_progress.emit("model-download-progress", &progress);
        }
    });

    match model_type {
        ModelType::Language => {
            let model = models::get_language_model(&model_id)
                .ok_or(CommandError::model_not_found(&model_id))?;
            if state.model_downloader.is_downloaded(&model) {
                tracing::info!("Model {} is already downloaded", model_id);
                return Ok(());
            }
            state
                .model_downloader
                .download(&model, model_type, status_tx, progress_tx)
                .await
                .external_err()?;
        }
        ModelType::Embedding => {
            let model = models::get_embedding_model(&model_id)
                .ok_or(CommandError::model_not_found(&model_id))?;
            if state.model_downloader.is_downloaded(&model) {
                tracing::info!("Model {} is already downloaded", model_id);
                return Ok(());
            }
            state
                .model_downloader
                .download(&model, model_type, status_tx, progress_tx)
                .await
                .external_err()?;
        }
        ModelType::Ocr => {
            let model =
                models::get_ocr_model(&model_id).ok_or(CommandError::model_not_found(&model_id))?;
            if state.model_downloader.is_downloaded(&model) {
                tracing::info!("Model {} is already downloaded", model_id);
                return Ok(());
            }
            state
                .model_downloader
                .download(&model, model_type, status_tx, progress_tx)
                .await
                .external_err()?;
        }
    }

    Ok(())
}

/// Snapshot of a provider's persistent state (unconfigured or ready).
///
/// Transient states (downloading, loading, failed) are carried by
/// `model-status-changed` events, not by this query.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderStatus {
    pub provider_type: Option<String>,
    pub model_id: Option<String>,
    pub ready: bool,
}

#[tauri::command]
pub async fn get_provider_status(
    model_type: ModelType,
    state: State<'_, AppState>,
) -> CommandResult<ProviderStatus> {
    Ok(match model_type {
        ModelType::Embedding => {
            let model_id = state.models.embedding_model_id().await;
            let ready = state.models.embedding_ready().await;
            ProviderStatus {
                provider_type: model_id.as_ref().map(|_| "local".to_string()),
                model_id,
                ready,
            }
        }
        ModelType::Language => {
            let config = state.models.chat_config().await;
            let ready = state.models.chat_ready().await;
            ProviderStatus {
                provider_type: config.as_ref().map(|c| c.provider_type().to_string()),
                model_id: config.as_ref().map(|c| c.model_id().to_string()),
                ready,
            }
        }
        ModelType::Ocr => {
            let model_id = state.models.ocr_model_id().await;
            let ready = state.models.ocr_ready().await;
            ProviderStatus {
                provider_type: model_id.as_ref().map(|_| "local".to_string()),
                model_id,
                ready,
            }
        }
    })
}

/// Get the currently configured model ID for a type
#[tauri::command]
pub async fn get_current_model(
    model_type: ModelType,
    state: State<'_, AppState>,
) -> CommandResult<Option<String>> {
    Ok(match model_type {
        ModelType::Language => state
            .models
            .chat_config()
            .await
            .map(|c| c.model_id().to_string()),
        ModelType::Embedding => state.models.embedding_model_id().await,
        ModelType::Ocr => state.models.ocr_model_id().await,
    })
}

/// Configure and load a model.
///
/// Emits `model-status-changed` events around the slow load so the frontend
/// state machine stays in sync (`loading` → `ready` or `failed`).
#[tauri::command]
pub async fn configure_model(
    model_type: ModelType,
    model_id: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    match model_type {
        ModelType::Language => configure_language_model_impl(model_id, app, state).await,
        ModelType::Embedding => configure_embedding_model_impl(model_id, app, state).await,
        ModelType::Ocr => configure_ocr_model_impl(model_id, app, state).await,
    }
}

fn emit_ready(app: &AppHandle, model_type: ModelType, id: &str) {
    use crate::core::ModelStatus;
    let _ = app.emit(
        "model-status-changed",
        &ModelStatus::Ready {
            model_type,
            model_id: id.to_string(),
        },
    );
}

fn emit_failed(app: &AppHandle, model_type: ModelType, id: &str, error: &str) {
    use crate::core::ModelStatus;
    let _ = app.emit(
        "model-status-changed",
        &ModelStatus::Failed {
            model_type,
            model_id: id.to_string(),
            error: error.to_string(),
        },
    );
}

async fn configure_language_model_impl(
    model_id: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    use crate::core::{LocalChatProvider, ProviderConfig, Settings};

    if let Some(ref id) = model_id {
        let model = models::get_language_model(id).ok_or(CommandError::model_not_found(id))?;

        tracing::info!(
            "Configuring local language model: {} ({})",
            model.name,
            model.id
        );

        let model_path = state
            .model_downloader
            .get_path(&model)
            .ok_or(CommandError::model_not_downloaded(id))?;

        let provider = LocalChatProvider::new(&model_path, &model);

        let provider_config = ProviderConfig::Local {
            model_id: id.clone(),
        };
        if let Err(e) = state
            .models
            .set_chat(Arc::new(provider), provider_config.clone())
            .await
        {
            let msg = format!("Failed to install provider: {}", e);
            emit_failed(&app, ModelType::Language, id, &msg);
            return Err(CommandError::internal(msg));
        }

        let mut settings = Settings::load(&state.config.settings_file);
        settings.provider = Some(provider_config);
        settings.save(&state.config.settings_file).storage_err()?;

        emit_ready(&app, ModelType::Language, id);
    } else {
        tracing::info!("Unloading chat provider");
        state.models.clear_chat().await;

        let mut settings = Settings::load(&state.config.settings_file);
        settings.provider = None;
        settings.save(&state.config.settings_file).storage_err()?;
    }

    Ok(())
}

async fn configure_embedding_model_impl(
    model_id: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    use crate::core::{LocalEmbeddingProvider, Settings};

    if let Some(ref id) = model_id {
        let model = models::get_embedding_model(id).ok_or(CommandError::model_not_found(id))?;

        tracing::info!(
            "Configuring embedding model: {} ({})",
            model.name,
            model.hf_repo_id
        );

        let provider = LocalEmbeddingProvider::new(id, &model.hf_repo_id, model.dimensions);

        {
            let index = &*state.search;
            let indexer_config = milli::update::IndexerConfig::default();
            search::configure_embedder(index, &indexer_config, "default", model.dimensions)
                .map_err(|e| {
                    CommandError::internal(format!("Failed to configure embedder in index: {}", e))
                })?;
        }

        if let Err(e) = state
            .models
            .set_embedding(Arc::new(provider), id.clone())
            .await
        {
            let msg = format!("Failed to install embedder: {}", e);
            emit_failed(&app, ModelType::Embedding, id, &msg);
            return Err(CommandError::internal(msg));
        }

        let mut settings = Settings::load(&state.config.settings_file);
        settings.embedding_model_id = Some(id.clone());
        settings.save(&state.config.settings_file).storage_err()?;

        emit_ready(&app, ModelType::Embedding, id);
    } else {
        tracing::info!("Disabling embedding model");

        {
            let index = &*state.search;
            let indexer_config = milli::update::IndexerConfig::default();
            if let Err(e) = search::remove_embedder(index, &indexer_config) {
                tracing::warn!("Failed to remove embedder from index: {}", e);
            }
        }

        state.models.clear_embedding().await;

        let mut settings = Settings::load(&state.config.settings_file);
        settings.embedding_model_id = None;
        settings.save(&state.config.settings_file).storage_err()?;
    }

    Ok(())
}

async fn configure_ocr_model_impl(
    model_id: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    use crate::core::{LocalOcrProvider, Settings};

    if let Some(ref id) = model_id {
        let model = models::get_ocr_model(id).ok_or(CommandError::model_not_found(id))?;

        tracing::info!(
            "Configuring OCR model: {} ({})",
            model.name,
            model.hf_repo_id
        );

        let provider = LocalOcrProvider::new(id, &model.hf_repo_id, &model.prompt);

        if let Err(e) = state.models.set_ocr(Arc::new(provider), id.clone()).await {
            let msg = format!("Failed to install OCR provider: {}", e);
            emit_failed(&app, ModelType::Ocr, id, &msg);
            return Err(CommandError::internal(msg));
        }

        let mut settings = Settings::load(&state.config.settings_file);
        settings.ocr_model_id = Some(id.clone());
        settings.save(&state.config.settings_file).storage_err()?;

        emit_ready(&app, ModelType::Ocr, id);

        // Drain any documents whose extract phase parked an `ocr_task`
        // entry while OCR was unconfigured. Phase 4 wires this method.
        state.pipeline.requeue_pending_ocr().await;
    } else {
        tracing::info!("Disabling OCR model");
        state.models.clear_ocr().await;

        let mut settings = Settings::load(&state.config.settings_file);
        settings.ocr_model_id = None;
        settings.save(&state.config.settings_file).storage_err()?;
    }

    Ok(())
}
