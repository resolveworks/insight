use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::agent::Conversation;

/// Summary of a conversation for listing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub updated_at: String,
}

/// List all conversation summaries from disk, sorted by most recent first
pub fn list_conversations(conversations_dir: &Path) -> Result<Vec<ConversationSummary>> {
    let mut summaries = Vec::new();

    if !conversations_dir.exists() {
        return Ok(summaries);
    }

    for entry in std::fs::read_dir(conversations_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "json") {
            match load_conversation(&path) {
                Ok(conv) => {
                    summaries.push(ConversationSummary {
                        id: conv.id,
                        title: conv.title,
                        updated_at: conv.updated_at,
                    });
                }
                Err(e) => {
                    tracing::warn!("Failed to load conversation {:?}: {}", path, e);
                }
            }
        }
    }

    // Sort by updated_at descending (most recent first)
    summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    Ok(summaries)
}

/// Load a full conversation from disk
pub fn load_conversation(path: &Path) -> Result<Conversation> {
    let content = std::fs::read_to_string(path).context("Failed to read conversation file")?;
    let conversation: Conversation =
        serde_json::from_str(&content).context("Failed to parse conversation JSON")?;
    Ok(conversation)
}

/// Save a conversation to disk
pub fn save_conversation(conversations_dir: &Path, conversation: &Conversation) -> Result<()> {
    let path = conversations_dir.join(format!("{}.json", conversation.id));
    let content =
        serde_json::to_string_pretty(conversation).context("Failed to serialize conversation")?;
    std::fs::write(&path, content).context("Failed to write conversation file")?;
    Ok(())
}

/// Get the path for a specific conversation
pub fn conversation_path(conversations_dir: &Path, id: &str) -> std::path::PathBuf {
    conversations_dir.join(format!("{}.json", id))
}
