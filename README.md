# Insight

**Local-first document search for journalists and newsrooms.**

A research project exploring how peer-to-peer technology can give journalists control over their documents without relying on cloud services.

## The Problem

Newsrooms accumulate documents—leaks, court filings, FOIA responses, research papers. Searching and sharing them usually means uploading to cloud services you don't control. That's a problem when sources trust you with sensitive material.

## What Insight Does

- **Search across documents** with full-text and semantic search (finds related concepts, not just keywords)
- **Share with colleagues** directly, laptop-to-laptop, no server required
- **Work offline** — your documents and search index live on your machine
- **Run a newsroom server** — same app in headless mode for always-on availability

## How It Works

Documents sync peer-to-peer using [iroh](https://iroh.computer/). Each machine builds its own search index locally using [milli](https://github.com/meilisearch/milli) (the engine behind Meilisearch). No central server sees your files.

```
You add a PDF → Text extracted → Indexed locally → Syncs to colleagues
Colleague searches → Their local index → Finds your document → Fetches from you
```

## Project Status

This is a **research project** exploring:

- Practical P2P sync for document workflows
- Local semantic search with embeddings
- Trust models for journalist collaboration

Not production-ready. Built to learn and prototype.

## Technology

| Layer    | Choice       | Why                                          |
| -------- | ------------ | -------------------------------------------- |
| App      | Tauri + Rust | Single binary, cross-platform, no Electron   |
| Frontend | Svelte 5     | Fast, minimal                                |
| P2P      | iroh         | Modern QUIC-based, handles NAT traversal     |
| Search   | milli        | Full-text + vector search, runs locally      |
| PDF      | lopdf        | Text extraction (opens in system PDF viewer) |

## Building

```bash
pnpm install
pnpm tauri dev          # Desktop app with hot reload
pnpm tauri build        # Release build (CPU)
```

GPU-accelerated builds (optional):

```bash
# NVIDIA (requires CUDA toolkit)
pnpm tauri build -- --features "cuda flash-attn cudnn"

# Apple Silicon
pnpm tauri build -- --features metal

# Intel MKL
pnpm tauri build -- --features mkl
```

Headless server mode:

```bash
cd src-tauri && cargo run -- --headless
```

## Who This Is For

- **Journalists** managing sensitive document collections
- **Newsrooms** wanting to share documents internally without cloud dependencies
- **Civic tech researchers** interested in P2P collaboration tools
- **Anyone** exploring alternatives to centralized document platforms

## License

[TBD]

---

_Built as a research exploration of local-first software for journalism._
