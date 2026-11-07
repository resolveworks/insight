# Architecture

Insight is built as a Tauri 2.0 desktop application with a Rust backend and Svelte frontend.

## Core Stack

| Component       | Library     | Purpose                                                                          |
| --------------- | ----------- | -------------------------------------------------------------------------------- |
| App framework   | Tauri 2.0   | Desktop app (Rust backend, web frontend)                                         |
| UI              | Svelte 5    | Frontend                                                                         |
| Styling         | Tailwind 4  | Utility-first CSS                                                                |
| Chat inference  | [rig]       | LLM framework: 27 vendor selections via a registry, agent runtime, tools, memory |
| P2P / Sync      | iroh        | Connections, NAT traversal, sync                                                 |
| Content storage | iroh-blobs  | Content-addressed file storage                                                   |
| Metadata sync   | iroh-docs   | CRDT key-value store for metadata                                                |
| Real-time       | iroh-gossip | Pub/sub for live updates                                                         |
| Search          | milli       | Full-text keyword search (BM25)                                                  |
| PDF text        | MuPDF       | Digital text extraction                                                          |

[rig]: https://github.com/0xPlaygrounds/rig

## Agent Architecture

```
User Query
    ↓
rig Agent (per request: preamble + tools + memory + max turns)
    ↓
Registry-built Model (one of 27 curated rig vendor selections)
    ↓
Tool Calling Loop (multi-turn)
    ↓
Synthesized Answer (with citations)
```

Inference is remote-only: the chat provider is configured in Settings as a canonical rig `ProviderId` selection (for example `openai/openai` or `anthropic/anthropic`) plus a model and optional API key / base URL. Insight exposes one curated selection per vendor from [rig](https://github.com/0xPlaygrounds/rig)'s provider registry — 27 vendors including OpenAI, Anthropic, Gemini, Groq, OpenRouter, and local servers like Ollama and llama.cpp. The registry currently registers 34 selections; Insight does not expose every one. Only the conversation—queries and retrieved excerpts—goes to the provider. Documents are stored, searched, and synced entirely on your machine.

The runtime (`crates/insight-core/src/runtime.rs`) builds a fresh rig agent per request: a stable preamble, four manual `Tool` implementations, and file-backed conversation memory. Each tool owns a clone of the per-run `ToolState` (search index, storage, active collections); Insight keeps these live handles on the tool instances rather than in `ToolContext`. The conversation's active collections are attached as a run document with `AgentRunner::document` on every turn, so a mid-conversation scope change applies to the next model call without touching stored history. `generate_title` and `predict_next_message` are bare-model completions over the same provider settings. Conversation history persists in rig's `DirStore` at `conversations/logs/{id}.jsonl`, with app metadata at `conversations/metadata/{id}.json`. Agent progress streams to the frontend as rig `MultiTurnStreamItem` JSON events; the Tauri command emits a single terminal `streamError` when the run fails.

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
	"units": [1500, 3200, 4800],
	"unit_kind": "page",
	"tags": ["research", "climate"],
	"created_at": "2024-01-15T10:30:00Z"
}
```

`units` are the addressable positions of the source, mapped onto the extracted text: for a PDF, the byte offset where each page ends. `unit_kind` names what one unit is, and is `null` when the source has no recorded unit structure (before extraction, or a format with no natural units). Page counts are derived from the table, so they are either known or absent — never a placeholder zero.

### Content-Addressed Storage

All file content (PDFs, extracted text) is stored in iroh-blobs using content-addressing:

- Files are identified by their BLAKE3 hash
- Duplicate files are automatically deduplicated
- Content can be verified for integrity

## Data Flow

### Local Import

1. User adds PDF to collection
2. **Store**: original PDF bytes written to `files/{id}/source` (iroh)
3. **Extract**: per-page text extracted with MuPDF (digital text only; the page/unit table is recorded), written to `files/{id}/text` alongside `files/{id}/meta`
4. **Index**: the watcher triggers the index worker, which chunks text (~1500 chars, ~200 overlap) and indexes chunks in milli for full-text search. Chunks store the byte span they occupy in the document text; page references are resolved from the document's units when a search hit is read

**No OCR**: extraction uses MuPDF's text layer only. Scanned PDFs (images of text) import fine but yield little or no extracted text and are not text-searchable.

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
├── settings.json       # settings v2
└── conversations/
    ├── logs/           # rig conversation history, {id}.jsonl
    └── metadata/       # app metadata, {id}.json
```

Chat provider configuration (settings v2: active `provider` plus per-selection `api_keys`) lives in `~/.local/share/insight/settings.json`. The active `family` and the `api_keys` entries use canonical `ProviderId` selection ids such as `openai/openai` and `anthropic/anthropic`. The legacy `conversations/{id}/messages.json` + `meta.json` layout is rejected, not migrated. On Windows, app data is under `%LOCALAPPDATA%\insight\`.
