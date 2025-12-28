# Insight

A local-first research agent for evidence-based journalism. Think Claude Code, but for documents and investigations.

## Why Insight?

Newsrooms have documents but no good way to search, analyze, and share them without relying on cloud services they don't trust. Insight combines local LLM inference with P2P document sync to give journalists an AI research assistant that runs entirely on their hardware.

## Key Features

- **Local-first**: All processing happens on your machine. No data leaves your computer unless you explicitly share it.
- **AI research agent**: Ask questions in natural language and get answers with citations from your documents.
- **P2P sync**: Share collections with colleagues directly, laptop-to-laptop, without a central server.
- **Offline capable**: Works without internet once models are downloaded.

## How It Works

```
User Query
    ↓
Local LLM (via mistralrs)
    ↓
Tool Calling Loop (search, read documents)
    ↓
Synthesized Answer (with citations)
```

The agent iteratively gathers evidence from your documents and synthesizes answers—all running locally.

## Quick Start

1. [Download Insight](https://github.com/resolveworks/insight/releases) for your platform
2. Create a collection and add documents
3. Start asking questions

See [Getting Started](./getting-started.md) for detailed setup instructions.
