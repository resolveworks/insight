# Architecture

Insight is built as a Tauri 2.0 desktop application with a Rust backend and Svelte frontend.

## Core Stack

| Component       | Library     | Purpose                                                        |
| --------------- | ----------- | -------------------------------------------------------------- |
| App framework   | Tauri 2.0   | Desktop app (Rust backend, web frontend)                       |
| UI              | Svelte 5    | Frontend                                                       |
| Styling         | Tailwind 4  | Utility-first CSS                                              |
| Chat inference  | [rig]       | LLM framework: provider registry, agent runtime, tools, memory |
| P2P / Sync      | iroh        | Connections, NAT traversal, sync                               |
| Content storage | iroh-blobs  | Content-addressed file storage                                 |
| Metadata sync   | iroh-docs   | CRDT key-value store for metadata                              |
| Real-time       | iroh-gossip | Pub/sub for live updates                                       |
| Search          | milli       | Full-text keyword search (BM25)                                |
| PDF text        | MuPDF       | Digital text extraction                                        |

[rig]: https://github.com/0xPlaygrounds/rig

## Agent Architecture

```
User Query
    ↓
rig Agent (per request: preamble + tools + memory + max turns)
    ↓
Registry-built Model (the configured rig provider selection)
    ↓
Tool Calling Loop (multi-turn)
    ↓
Synthesized Answer (with citations)
```

Inference is remote-only: the chat provider is configured in Settings as a canonical rig `ProviderId` selection (for example `openai/openai` or `anthropic/anthropic`) plus a model and optional API key / base URL. Insight exposes every selection [rig](https://github.com/0xPlaygrounds/rig)'s provider registry registers — 34 selections across 27 vendors, including OpenAI, Anthropic, Gemini, Groq, OpenRouter, and local servers like Ollama and llama.cpp. Only the conversation—queries and retrieved excerpts—goes to the provider. Documents are stored, searched, and synced entirely on your machine.

The runtime (`crates/insight-core/src/runtime.rs`) builds a fresh rig agent per request: a stable preamble, four manual `Tool` implementations, and file-backed conversation memory. Each tool owns a clone of the per-run `ToolState` (search index, storage, the conversation's collection); Insight keeps these live handles on the tool instances rather than in `ToolContext`. The conversation's collection is attached as an `active_collection` run document naming it on every turn; the preamble and tools are scoped to that one collection. Title generation runs after the first successful turn, falling back to rig-memory's `title_of` of the first user message. Conversation history persists in a per-conversation rig `DirStore`: `conversations/{collection_id}/{conversation_id}/logs/{conversation_id}.jsonl` for the session log and `conversations/{collection_id}/{conversation_id}/files/meta.json` for `rig_memory::SessionMeta` (title and updated). Rust normalizes stored history and the live agent stream into the app-owned chat contract (`crates/insight-core/src/chat.rs`): `TranscriptItem` blocks keyed by message and block, delivered as `ChatEvent`s — item upserts and removals while the run streams, then a terminal `run_finished` carrying the committed transcript. `load_conversation` returns the same items for stored history. When the run fails, the command sends a single terminal `run_failed` event through that same channel.

The agent has four typed tools (search, read_chunk, list_documents, get_collection_terms) built on milli search and iroh storage, all scoped to the conversation's collection. It iteratively gathers evidence to answer questions, citing sources along the way. There is no direct user-facing search—all document retrieval happens through the agent.

## Conversation Model

A conversation belongs to exactly one evidence collection, fixed at creation. There is no multi-collection scope and no mid-conversation scope change: a different collection means a different conversation. Conversations live only on the local device and do not sync; sharing a collection shares its documents, not its chats.

The Tauri command layer (`src-tauri/src/commands/conversations.rs`) scopes every conversation command by collection: `start_chat(collectionId)`, `list_conversations(collectionId)`, `load_conversation(collectionId, conversationId)`, `send_message(collectionId, conversationId, message)`, and `delete_conversation(collectionId, conversationId)`. Deleting a collection also deletes its conversations.

In the UI, each collection is one workspace at `/collections/{collectionId}` with Research and Documents tabs; opening a collection lands on Research, with that collection's conversations in a sidebar beside the chat. The top-level tabs are Collections and Settings, and `/` redirects to Collections.

## Data Model

### Collections as Namespaces

Each collection is an iroh-docs namespace. Sharing a collection means sharing namespace access.

```
Namespace: 7f3a8b2c... ("Climate Research")
│
├── files/abc123/meta                         → document metadata (JSON)
├── files/abc123/source                       → original PDF bytes
├── files/abc123/extractions/mupdf-pdf-text@1 → extraction record (JSON)
├── files/def456/meta                         → document metadata (JSON)
├── files/def456/source                       → original PDF bytes
├── files/def456/extractions/mupdf-pdf-text@1 → extraction record (JSON)
├── _hash_index/{hash}                        → duplicate detection index
└── _collection                               → collection settings
```

A document's source is immutable. Text is a derivation of that source: a document may have many extractions, each produced by a parser and stored under `{name}@{version}`. The extraction record points at a text blob and carries the source unit mapping:

```json
{
	"parser": { "name": "mupdf-pdf-text", "version": "1" },
	"text_hash": "...",
	"unit_kind": "page",
	"units": [{ "number": 1, "spans": [{ "start": 0, "end": 1500 }] }]
}
```

`text_hash` is hex because iroh's `Hash` has no serde representation. `units` are the source's addressable positions with the byte spans of the extraction text attributed to each: for a PDF, one unit per page numbered from 1. A page with no text is a unit with empty `spans`; units are ordered by number, and spans within a unit are ascending and non-overlapping. `unit_kind` is `null` when the parser records no unit structure.

### Document Metadata

```json
{
	"id": "abc123",
	"name": "paper.pdf",
	"file_type": "application/pdf",
	"extraction": { "name": "mupdf-pdf-text", "version": "1" },
	"tags": ["research", "climate"],
	"created_at": "2024-01-15T10:30:00Z"
}
```

`extraction` names the document's canonical extraction — the one search indexes. It is `null` before any extraction exists, and the document then has no text and an unknown page count. Page counts are derived from the canonical extraction's units, so they are either known or absent — never a placeholder zero. Documents that predate extractions are not migrated; they have no canonical extraction until re-imported.

### Content-Addressed Storage

All file content (PDFs and extraction text) is stored in iroh-blobs using content-addressing:

- Files are identified by their BLAKE3 hash
- Duplicate sources are detected by hash and rejected on import
- Content can be verified for integrity

## Data Flow

### Local Import

1. User adds PDF to collection
2. **Store**: original PDF bytes written to `files/{id}/source` (iroh), with `files/{id}/meta` and the `_hash_index` duplicate check
3. **Extract**: MuPDF produces an extraction (digital text only; one unit per page with its text spans) written to `files/{id}/extractions/{parser_key}`. The first extraction becomes the document's canonical extraction
4. **Index**: the watcher triggers the index worker, which chunks the canonical extraction's text (~1500 chars, ~200 overlap) and indexes chunks in milli for full-text search. Chunks store the byte span they occupy in the extraction text; page references are not stored on the chunk

Selecting a different extraction as canonical changes what search indexes. Changed source content is a new document, never a new version of the same one.

**No OCR**: extraction uses MuPDF's text layer only. Scanned PDFs (images of text) import fine but yield little or no extracted text and are not text-searchable.

### On Sync

When document entries arrive from a peer, iroh-docs automatically syncs the entry content blobs. The watcher reacts to `files/*/meta` and extraction entries (local or remote) by queueing indexing for the document:

1. The index worker reads the canonical extraction and its text blob
2. Chunks of that text are indexed in milli for search
3. If an extraction entry arrives before its content, the watcher records the content hash and queues indexing when the matching `ContentReady` event arrives; the index worker skips cleanly until the extraction and text are available
4. Source file at `files/{id}/source` is available immediately

## What Syncs vs What's Local

| Data            | Syncs                       | Stored in  |
| --------------- | --------------------------- | ---------- |
| PDF files       | Yes                         | iroh-blobs |
| Extractions     | Yes                         | iroh-blobs |
| File metadata   | Yes                         | iroh-docs  |
| Collection info | Yes                         | iroh-docs  |
| Search index    | No (built from synced text) | milli      |
| Conversations   | No                          | local disk |

Search is local keyword indexing over the canonical extraction's text: each peer builds its own milli index. Nothing beyond the raw document data is derived or replicated.

## Local Storage

```
~/.local/share/insight/
├── iroh/               # iroh data (blobs, docs)
├── search/             # milli index
├── settings.json       # settings v2
└── conversations/
    └── {collection_id}/
        └── {conversation_id}/
            ├── logs/{conversation_id}.jsonl   # rig session log
            └── files/meta.json                # rig SessionMeta (title, updated ms)
```

Collection ids are canonical namespace-id strings. Each conversation is one rig `DirStore`; `SessionMeta` is the only conversation metadata, with no separate metadata sidecar and no `logs/` at the root. Chat provider configuration (settings v2: a single `provider` object with `family`, `model`, and optional `api_key` and `base_url`) lives in `~/.local/share/insight/settings.json`. The `family` is a canonical `ProviderId` selection id such as `openai/openai` or `anthropic/anthropic`. There is no migration from earlier conversation layouts; they are not read. On Windows, app data is under `%LOCALAPPDATA%\insight\`.
