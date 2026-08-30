# Insight

**AI-powered research agent for journalists and newsrooms.**

Search documents, ask questions, get answers with citations. Your documents stay local; chat runs on your configured AI provider.

## What Insight Does

- **Ask questions, get answers** — AI agent searches your documents and synthesizes answers with citations
- **Documents stay local** — your files are stored, indexed, and searched on your machine, never uploaded
- **Choice of provider** — 25+ LLM providers for chat (OpenAI, Anthropic, Gemini, Groq, local servers like Ollama), configured with your own API key
- **Share with colleagues** — sync collections peer-to-peer, no server required

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

Your documents are indexed locally using [milli](https://github.com/meilisearch/milli) for full-text keyword search (BM25). The AI agent—running on the [rig](https://github.com/0xPlaygrounds/rig) framework with your configured provider—searches and reads documents to answer your questions. Only your conversation (questions and retrieved excerpts) goes to the provider; the documents themselves never leave your machine.

Collections sync peer-to-peer using [iroh](https://iroh.computer/), so collaborators get full copies of documents and extracted text. Each peer builds its own search index locally.

## Project Status

This is a **research project** exploring:

- AI-assisted document research for journalism
- Practical P2P sync for document workflows
- Agent-driven search with full-text retrieval
- Trust models for journalist collaboration

Not production-ready. Built to learn and prototype.

## Technology

| Layer    | Choice             | Why                                        |
| -------- | ------------------ | ------------------------------------------ |
| App      | Tauri + Rust       | Single binary, cross-platform, no Electron |
| Frontend | Svelte 5           | Fast, minimal                              |
| LLM      | rig (25+ providers) | Remote inference via API (or local servers), user-configured |
| P2P      | iroh               | Modern QUIC-based, handles NAT traversal   |
| Search   | milli              | Full-text keyword search (BM25), local     |
| PDF      | lopdf              | Digital text extraction                    |

## Building

```bash
pnpm install
pnpm tauri dev          # Desktop app with hot reload
pnpm tauri build        # Release build
```

## Who This Is For

- **Journalists** managing document collections for investigations
- **Newsrooms** wanting AI-assisted research without uploading documents
- **Investigators** who need to analyze and search large document sets
- **Researchers** exploring AI tools for document-based work

---

_A research exploration of AI-assisted journalism tools._
