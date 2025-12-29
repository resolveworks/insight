# Insight

**AI-powered research agent for journalists and newsrooms.**

Search documents, ask questions, get answers with citations. Use cloud AI for convenience or run everything locally for maximum privacy.

## What Insight Does

- **Ask questions, get answers** — AI agent searches your documents and synthesizes answers with citations
- **Your choice of AI** — use cloud models (Anthropic, OpenAI) or run local models on-device
- **Documents stay local** — your files are processed on your machine, never uploaded
- **Share with colleagues** — sync collections peer-to-peer, no server required
- **Work offline** — with local models, everything runs without internet

## How It Works

```
You: "What do these documents say about the 2019 contract?"
    ↓
AI agent searches your documents
    ↓
Reads relevant files, synthesizes answer
    ↓
"According to the March 2019 filing [doc: contract-v2.pdf]..."
```

Your documents are indexed locally using [milli](https://github.com/meilisearch/milli) for hybrid full-text and vector search. The AI agent—powered by cloud models or a local LLM via [mistralrs](https://github.com/EricLBuehler/mistral.rs)—searches and reads documents to answer your questions.

Collections sync peer-to-peer using [iroh](https://iroh.computer/), including pre-computed embeddings so collaborators don't redo the work.

## Project Status

This is a **research project** exploring:

- AI-assisted document research for journalism
- Practical P2P sync for document workflows
- Agent-driven search with hybrid full-text and vector retrieval
- Trust models for journalist collaboration

Not production-ready. Built to learn and prototype.

## Technology

| Layer    | Choice       | Why                                          |
| -------- | ------------ | -------------------------------------------- |
| App      | Tauri + Rust | Single binary, cross-platform, no Electron   |
| Frontend | Svelte 5     | Fast, minimal                                |
| LLM      | mistralrs    | Local inference option (GGUF models)         |
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

- **Journalists** managing document collections for investigations
- **Newsrooms** wanting AI-assisted research with privacy options
- **Investigators** who need to analyze and search large document sets
- **Researchers** exploring AI tools for document-based work

---

_A research exploration of AI-assisted journalism tools._
