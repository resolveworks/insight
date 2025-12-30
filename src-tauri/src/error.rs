//! Command error types for Tauri frontend communication
//!
//! Provides structured, type-safe errors that serialize to `{"code": "...", "message": "..."}`.

use serde::Serialize;

/// Errors returned by Tauri commands
///
/// Each variant serializes with a snake_case `code` field for frontend matching.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum CommandError {
    // Validation errors
    InvalidCollectionId { message: String },
    InvalidUtf8 { message: String },

    // Not found errors
    DocumentNotFound { message: String },
    TextNotFound { message: String },
    CollectionNotFound { message: String },
    ConversationNotFound { message: String },
    ModelNotFound { message: String, model_id: String },

    // Configuration errors
    ModelNotDownloaded { message: String, model_id: String },
    EmbedderNotConfigured { message: String },
    ProviderNotConfigured { message: String },

    // Operation errors
    StorageError { message: String },
    ExternalError { message: String },
    InternalError { message: String },
}

impl CommandError {
    pub fn invalid_collection_id() -> Self {
        Self::InvalidCollectionId {
            message: "Invalid collection ID".to_string(),
        }
    }

    pub fn document_not_found() -> Self {
        Self::DocumentNotFound {
            message: "Document not found".to_string(),
        }
    }

    pub fn text_not_found() -> Self {
        Self::TextNotFound {
            message: "Text content not found".to_string(),
        }
    }

    pub fn collection_not_found() -> Self {
        Self::CollectionNotFound {
            message: "Collection not found".to_string(),
        }
    }

    pub fn conversation_not_found() -> Self {
        Self::ConversationNotFound {
            message: "Conversation not found".to_string(),
        }
    }

    pub fn model_not_found(model_id: impl Into<String>) -> Self {
        let model_id = model_id.into();
        Self::ModelNotFound {
            message: format!("Model not found: {}", model_id),
            model_id,
        }
    }

    pub fn model_not_downloaded(model_id: impl Into<String>) -> Self {
        let model_id = model_id.into();
        Self::ModelNotDownloaded {
            message: format!("Model not downloaded: {}", model_id),
            model_id,
        }
    }

    pub fn embedder_not_configured() -> Self {
        Self::EmbedderNotConfigured {
            message: "Embedder not configured. Please configure an embedding model first."
                .to_string(),
        }
    }

    pub fn provider_not_configured() -> Self {
        Self::ProviderNotConfigured {
            message: "No chat provider configured".to_string(),
        }
    }

    pub fn storage(message: impl Into<String>) -> Self {
        Self::StorageError {
            message: message.into(),
        }
    }

    pub fn external(message: impl Into<String>) -> Self {
        Self::ExternalError {
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::InternalError {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCollectionId { message } => write!(f, "{}", message),
            Self::InvalidUtf8 { message } => write!(f, "{}", message),
            Self::DocumentNotFound { message } => write!(f, "{}", message),
            Self::TextNotFound { message } => write!(f, "{}", message),
            Self::CollectionNotFound { message } => write!(f, "{}", message),
            Self::ConversationNotFound { message } => write!(f, "{}", message),
            Self::ModelNotFound { message, .. } => write!(f, "{}", message),
            Self::ModelNotDownloaded { message, .. } => write!(f, "{}", message),
            Self::EmbedderNotConfigured { message } => write!(f, "{}", message),
            Self::ProviderNotConfigured { message } => write!(f, "{}", message),
            Self::StorageError { message } => write!(f, "{}", message),
            Self::ExternalError { message } => write!(f, "{}", message),
            Self::InternalError { message } => write!(f, "{}", message),
        }
    }
}

impl std::error::Error for CommandError {}

// Conversion from anyhow::Error (used by insight-core)
impl From<anyhow::Error> for CommandError {
    fn from(err: anyhow::Error) -> Self {
        Self::internal(err.to_string())
    }
}

// Conversion from std::io::Error
impl From<std::io::Error> for CommandError {
    fn from(err: std::io::Error) -> Self {
        Self::storage(err.to_string())
    }
}

/// Result type alias for commands
pub type CommandResult<T> = Result<T, CommandError>;

/// Extension trait for converting Results to CommandResult
pub trait ResultExt<T> {
    fn storage_err(self) -> CommandResult<T>;
    fn external_err(self) -> CommandResult<T>;
    fn internal_err(self) -> CommandResult<T>;
}

impl<T, E: std::fmt::Display> ResultExt<T> for Result<T, E> {
    fn storage_err(self) -> CommandResult<T> {
        self.map_err(|e| CommandError::storage(e.to_string()))
    }

    fn external_err(self) -> CommandResult<T> {
        self.map_err(|e| CommandError::external(e.to_string()))
    }

    fn internal_err(self) -> CommandResult<T> {
        self.map_err(|e| CommandError::internal(e.to_string()))
    }
}
