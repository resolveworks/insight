# Getting Started

## Installation

Download the latest release for your platform from the [releases page](https://github.com/resolveworks/insight/releases).

### System Requirements

- **macOS**: Apple Silicon (M1/M2/M3) recommended for best performance
- **Windows**: x64, Windows 10 or later
- **Linux**: x64, modern distribution with glibc 2.31+

### Storage Requirements

- ~5 GB for the default model (Qwen3 8B Q4_K_M)
- Additional space for your documents and search index

## Models

Insight uses local AI models that need to be downloaded before use. When you first use a feature that requires a model (chat or document processing), you'll be prompted to download it.

Models are stored in the HuggingFace cache:

- **macOS/Linux**: `~/.cache/huggingface/hub/`
- **Windows**: `%USERPROFILE%\.cache\huggingface\hub\`

The default language model is ~5 GB. An embedding model (~1.2 GB) is also needed for document processing.

## Creating Your First Collection

1. Click **New Collection**
2. Give it a name (e.g., "Climate Research")
3. Drag and drop PDF files into the collection

Insight will:

- Extract text from each PDF
- Generate embeddings for the AI agent

## Asking Questions

Once documents are processed, use the chat interface to ask questions:

- "What are the main findings about carbon emissions?"
- "Find all mentions of renewable energy targets"
- "Summarize the methodology used in these studies"

The agent will find relevant passages in your documents and synthesize an answer with citations.
