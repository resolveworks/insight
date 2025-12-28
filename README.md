# Insight

**Local-first research agent for journalists and newsrooms.**

An AI-powered research assistant that runs entirely on your machine. Search documents, ask questions, get answers with citations—no cloud required.

## The Problem

Newsrooms accumulate documents—leaks, court filings, FOIA responses, research papers. Searching and analyzing them usually means uploading to cloud services you don't control. That's a problem when sources trust you with sensitive material.

## What Insight Does

- **Ask questions, get answers** — AI assistant searches your documents and synthesizes answers with citations
- **Batteries included** — local LLM runs on-device, no API keys needed
- **Search across documents** — full-text and semantic search (finds related concepts, not just keywords)
- **Share with colleagues** — sync directly, laptop-to-laptop, no server required
- **Work offline** — your documents, search index, and AI run on your machine

## How It Works

Documents sync peer-to-peer using [iroh](https://iroh.computer/). Embeddings sync too—keyed by model, so peers using the same embedding model share computation. Each machine builds its own search index from synced data using [milli](https://github.com/meilisearch/milli). A local LLM (via [mistralrs](https://github.com/EricLBuehler/mistral.rs)) can search and read documents to answer your questions.

```
You: "What do these documents say about the 2019 contract?"
    ↓
Local LLM searches your documents
    ↓
Reads relevant files, synthesizes answer
    ↓
"According to the March 2019 filing [doc: contract-v2.pdf]..."
```

With local models, no data leaves your machine. Remote model providers will also be supported for users who prefer them.

## Project Status

This is a **research project** exploring:

- AI-assisted document research with local models
- Practical P2P sync for document workflows
- Local semantic search with embeddings
- Trust models for journalist collaboration

Not production-ready. Built to learn and prototype.

## Technology

| Layer    | Choice       | Why                                          |
| -------- | ------------ | -------------------------------------------- |
| App      | Tauri + Rust | Single binary, cross-platform, no Electron   |
| Frontend | Svelte 5     | Fast, minimal                                |
| LLM      | mistralrs    | Local inference, GGUF models                 |
| P2P      | iroh         | Modern QUIC-based, handles NAT traversal     |
| Search   | milli        | Full-text + vector search, runs locally      |
| PDF      | lopdf        | Text extraction (opens in system PDF viewer) |

## Building

```bash
pnpm install
pnpm tauri dev          # Desktop app with hot reload
pnpm tauri build        # Release build (CPU)
```

GPU-accelerated builds (faster inference):

```bash
# NVIDIA (requires CUDA toolkit)
pnpm tauri build -- --features cuda

# Apple Silicon
pnpm tauri build -- --features metal
```

## Who This Is For

- **Journalists** managing sensitive document collections
- **Newsrooms** wanting AI-assisted research without cloud dependencies
- **Investigators** who need to search and analyze large document sets
- **Anyone** exploring local-first AI tools for research

## License

[TBD]

---

_Built as a research exploration of local-first AI for journalism._
