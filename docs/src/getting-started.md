# Getting Started

## Download

{{#include generated/downloads.html}}

### Which file do I download?

- **macOS**: Download the `.dmg` file. Choose `aarch64` for newer Macs (M1/M2/M3/M4) or `x64` for older Intel Macs.
- **Windows**: Download the `.exe` installer. (IT departments may prefer the `.msi`.)
- **Linux**: Download `.deb` for Ubuntu/Debian, `.rpm` for Fedora/Red Hat, or `.AppImage` to run on any distribution.

## Configure an AI Provider

Insight uses OpenAI or Anthropic for chat. Add an API key from either provider in Settings to get started. This works well on any computer.

Your documents are always processed and stored locally—only your questions and the AI's answers go through the provider.

**You don't need the biggest model.** Smaller models like GPT-5 mini or Claude Haiku work well with Insight's agent—they're faster, cheaper, and handle document research tasks effectively. The agent harness does the heavy lifting of breaking down queries and gathering evidence, so even lightweight models produce good results.

## Create a Collection

Collections are folders for organizing your documents. You might create one for each investigation or story you're working on.

1. Click **New Collection**
2. Give it a name (e.g., "City Budget Investigation")
3. Drag and drop PDF files into the collection

Insight will process each document so the AI can search and read them. Note: text extraction works on digital PDFs; scanned PDFs (images of text) won't be text-searchable.

## Ask Questions

Once your documents are processed, just ask questions in plain language:

- "What was the total spending on consultants?"
- "Find all mentions of Company X"
- "Summarize the main findings from these reports"

The AI will search through your documents, find relevant passages, and give you an answer with citations you can click to see the original source.
