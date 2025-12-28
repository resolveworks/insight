# Getting Started

## Download

{{#include generated/downloads.html}}

### System Requirements

- **macOS**: Apple Silicon (M1/M2/M3) or Intel
- **Windows**: Windows 10 or later
- **Linux**: Most modern distributions

## Choosing an AI Model

Insight can use either local AI models or cloud services for chat. Your documents are always processed locally—only your questions and the AI's answers go through the cloud if you choose that option.

### Cloud Models (Recommended for most users)

If you have an API key from Anthropic or OpenAI, you can use their models. This is the easiest option and works well on any computer.

### Local Models

Running AI locally requires a powerful computer—ideally with a dedicated GPU (NVIDIA or Apple Silicon). On a typical laptop without a GPU, responses will be slow.

If you choose local models, Insight will download about 5 GB for the AI model on first use.

## Create a Collection

Collections are folders for organizing your documents. You might create one for each investigation or story you're working on.

1. Click **New Collection**
2. Give it a name (e.g., "City Budget Investigation")
3. Drag and drop PDF files into the collection

Insight will process each document so the AI can search and read them.

## Ask Questions

Once your documents are processed, just ask questions in plain language:

- "What was the total spending on consultants?"
- "Find all mentions of Company X"
- "Summarize the main findings from these reports"

The AI will search through your documents, find relevant passages, and give you an answer with citations you can click to see the original source.
