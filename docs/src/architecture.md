# Architecture

Insight is built as a Tauri 2.0 desktop application with a Rust backend and Svelte frontend.

## Core Stack

| Component       | Library                 | Purpose                                      |
| --------------- | ----------------------- | -------------------------------------------- |
| App framework   | Tauri 2.0               | Desktop app (Rust backend, web frontend)     |
| UI              | Svelte 5                | Frontend                                     |
| Styling         | Tailwind 4              | Utility-first CSS                            |
| Chat inference  | [rig]                 | LLM framework: 25+ provider families, agent runtime, tools, memory |
| P2P / Sync      | iroh                    | Connections, NAT traversal, sync             |
| Content storage | iroh-blobs              | Content-addressed file storage               |
| Metadata sync   | iroh-docs               | CRDT key-value store for metadata            |
| Real-time       | iroh-gossip             | Pub/sub for live updates                     |
| Search          | milli                   | Full-text keyword search (BM25)              |
| PDF text        | lopdf                   | Digital text extraction                      |

[rig]: https://github.com/0xPlaygrounds/rig

## Agent Architecture

```
User Query
    ↓
rig Agent (per request: preamble + tools + memory + max turns)
    ↓
Registry-built Model (any supported rig provider family)
    ↓
Tool Calling Loop (multi-turn)
    ↓
Synthesized Answer (with citations)
```

Inference is remote-only: the chat provider is configured in Settings (family + model + optional API key / base URL). Insight supports every chat-completion provider family offered by [rig](https://github.com/0xPlaygrounds/rig) — OpenAI, Anthropic, Gemini, Groq, OpenRouter, and local servers like Ollama and llama.cpp — through a data-driven registry. Only the conversation—queries and retrieved excerpts—goes to the provider. Documents are stored, searched, and synced entirely on your machine.

The runtime (`crates/insight-core/src/runtime.rs`) builds a fresh rig agent per request: a stable preamble, the document tools, file-backed conversation memory, and a scope hook that injects the conversation's active collections as context on every turn. `generate_title` and `predict_next_message` are bare-model completions over the same provider settings. Conversation history persists as rig messages plus a metadata sidecar under `conversations/{id}/`. Agent progress streams to the frontend as rig `MultiTurnStreamItem` JSON events.

The agent has four typed tools (search, read_chunk, list_documents, get_collection_terms) built on milli search and iroh storage. It iteratively gathers evidence to answer questions, citing sources along the way. There is no direct user-facing search—all document retrieval happens through the agent.

## Data Model

### Collections as Namespaces

Each collection is an iroh-docs namespace. Sharing a collection means sharing namespace access.

```
Namespace: 7f3a8b2c... ("Climate Research")
│
├── files/abc123/meta     → document metadata (JSON)
├── files/abc123/text     → extracted text
├── files/abc123/source   → original PDF bytes
├── files/def456/meta     → document metadata (JSON)
├── files/def456/text     → extracted text
├── files/def456/source   → original PDF bytes
├── _hash_index/{hash}    → duplicate detection index
└── _collection           → collection settings
```

### Document Metadata

```json
{
	"id": "abc123",
	"name": "paper.pdf",
	"file_type": "application/pdf",
	"page_count": 42,
	"tags": ["research", "climate"],
	"created_at": "2024-01-15T10:30:00Z",
	"page_boundaries": [0, 1500, 3200]
}
```

### Content-Addressed Storage

All file content (PDFs, extracted text) is stored in iroh-blobs using content-addressing:

- Files are identified by their BLAKE3 hash
- Duplicate files are automatically deduplicated
- Content can be verified for integrity

## Data Flow

### Local Import

1. User adds PDF to collection
2. **Store**: original PDF bytes written to `files/{id}/source` (iroh)
3. **Extract**: per-page text extracted with lopdf (digital text only; page boundaries recorded), written to `files/{id}/text` alongside `files/{id}/meta`
4. **Index**: the watcher triggers the index worker, which chunks text (~1500 chars, ~200 overlap, page-mapped) and indexes chunks in milli for full-text search

**No OCR**: extraction uses lopdf only. Scanned PDFs (images of text) import fine but yield little or no extracted text and are not text-searchable.

### On Sync

When document entries arrive from a peer, iroh-docs automatically syncs the entry content blobs. The SyncWatcher listens for `files/*/text` entries and triggers indexing:

1. Text is already available at `files/{id}/text` (synced by iroh)
2. Chunks are indexed in milli for search
3. Source file at `files/{id}/source` is available immediately

## What Syncs vs What's Local

| Data            | Syncs                       | Stored in  |
| --------------- | --------------------------- | ---------- |
| PDF files       | Yes                         | iroh-blobs |
| Extracted text  | Yes                         | iroh-blobs |
| File metadata   | Yes                         | iroh-docs  |
| Collection info | Yes                         | iroh-docs  |
| Search index    | No (built from synced text) | milli      |

Search is local keyword indexing over synced text: each peer builds its own milli index. Nothing beyond the raw document data is derived or replicated.

## Local Storage

```
~/.local/share/insight/
├── iroh/               # iroh data (blobs, docs)
├── search/             # milli index
└── conversations/      # chat history
```

Chat provider configuration (settings v2: active `provider` plus per-family `api_keys`) lives in `~/.local/share/insight/settings.json`. Conversations are stored under `conversations/{id}/` as `messages.json` (rig message history) and `meta.json` (title, linked collections). On Windows, app data is under `%LOCALAPPDATA%\insight\`.
