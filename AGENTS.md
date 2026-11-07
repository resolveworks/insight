# Insight - Local-First Research Agent

A local-first research agent for evidence-based journalism. Think Claude Code, but for documents and investigations.

## Vision

Newsrooms have documents but no good way to search, analyze, and share them without relying on cloud services they don't trust. Insight keeps document storage, search, and sync entirely local — while chat runs on a user-configured remote LLM provider (any selection rig registers: OpenAI, Anthropic, Gemini, Groq, local servers like Ollama/llama.cpp, ...). Only the conversation goes to the provider; documents never leave your machine.

## Core Stack

| Component       | Library     | Purpose                                                                       |
| --------------- | ----------- | ----------------------------------------------------------------------------- |
| App framework   | Tauri 2.0   | Desktop app (Rust backend, web frontend)                                      |
| UI              | Svelte 5    | Frontend                                                                      |
| Styling         | Tailwind 4  | Utility-first CSS (no theme() in component styles)                            |
| Chat inference  | [rig]       | LLM framework: provider registry, model catalog, agent runtime, tools, memory |
| P2P / Sync      | iroh        | Connections, NAT traversal, sync                                              |
| Content storage | iroh-blobs  | Content-addressed file storage                                                |
| Metadata sync   | iroh-docs   | CRDT key-value store for metadata                                             |
| Real-time       | iroh-gossip | Pub/sub for live updates                                                      |
| Search          | milli       | Full-text keyword search (BM25)                                               |
| PDF text        | MuPDF       | Digital text extraction                                                       |

[rig]: https://github.com/0xPlaygrounds/rig

The rig dependency tracks released crates.io versions (currently v0.46.0). A local reference checkout lives at `~/Projects/rig`, checked out at the matching tag (`v0.46.0`) — use it as the API reference for all rig types; see `~/Projects/rig/AGENTS.md` and the examples under `~/Projects/rig/examples/` for style.

## Architecture

### Agent Architecture

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

Key pieces, all in `crates/insight-core/`:

- **`providers/`** — a thin surface over rig's registry and built-in model catalog; nothing vendor-specific lives here. `selections()` lists every selection rig registers (`openai/openai`, `ollama/openai`, …), `models()` lists the chat models rig's catalog files under a selection's vendor (deprecated and non-chat rows dropped, rig's display names), and `build_model()` resolves the selection, applies the credential/endpoint rules rig declares, and returns a `DynModel<Completion>` built from the selection's `ProviderConfig` and catalog facts. Model listing is offline: it reads the catalog, it does not call the provider.
- **`runtime.rs`** — `send_message` builds a fresh rig agent **per request**: a stable preamble (`BASE_SYSTEM_PROMPT`) + the 4 document tools (manual `rig::tool::Tool` implementations) + `FileConversationMemory` + `default_max_turns(100)`. Each tool owns a clone of the per-run `ToolState` (index, storage, the conversation's collection); Insight keeps these live handles on the tool instances rather than in `ToolContext`. The conversation's collection is attached as an `active_collection` run document naming it on every turn; the preamble and tools are scoped to that one collection. The multi-turn stream is forwarded to the frontend as JSON `MultiTurnStreamItem` events on `agent-event-{conversation_id}`; cancellation drops the stream via a `CancellationToken`. Stream failures return as `Err`; the Tauri command emits the single terminal `streamError`. Title generation runs after the first successful turn, falling back to rig-memory's `title_of` of the first user message. `predict_next_message` is a bare-model completion (no tools, no agent loop) over the same provider settings.
- **`memory.rs`** — `FileConversationMemory` keeps one rig_memory `DirStore` per conversation, under its collection: `conversations/{collection_id}/{conversation_id}/` holds the rig session log at `logs/{conversation_id}.jsonl` and `files/meta.json`. It does not implement a `ConversationMemory` trait. `rig_memory::SessionMeta` is the only conversation metadata: title and updated (milliseconds since the epoch).
- **`tools/`** — manual `rig::tool::Tool` implementations over milli search + iroh storage; each owns a clone of the per-run `ToolState` (index, storage, the conversation's collection).

Inference is remote-only: the chat provider is configured in Settings. Documents never leave the machine — only the conversation (queries and retrieved excerpts) is sent to the configured provider.

### Conversation Commands

The Tauri command layer (`src-tauri/src/commands/conversations.rs`) scopes every conversation command by collection: `start_chat(collectionId)`, `list_conversations(collectionId)`, `load_conversation(collectionId, conversationId)`, `send_message(collectionId, conversationId, message)`, `predict_next_message(collectionId, conversationId)`, and `delete_conversation(collectionId, conversationId)`. Deleting a collection also deletes its conversations.

### Event Flow

`send_message` streams rig `MultiTurnStreamItem`s; the Tauri command layer (`src-tauri/src/commands/conversations.rs`) serializes them and emits them on `agent-event-{id}`. When the run fails, the runtime returns `Err` and the command emits the single terminal `{"type":"streamError","message"}` item.

The frontend renders ONE view model for both stored history and the live run. A pure fold (`src/lib/chat/transcript.ts`, `applyItem`) accumulates the stream into a `LiveRun` shaped like the stored transcript (chronological segments; a tool result closes the current assistant turn), and `buildTranscript` normalizes stored + live into a flat `TranscriptItem[]` (the conversations store exposes it as `getTranscript()`). Live-only state is data on the items (`provisional`, `executing`, `streaming` flags), so `Chat.svelte` has a single render path: a run committing swaps the provisional tail for the `finalResponse` payload's `messages` synchronously (tracked by the store's `runBase`) and the flags flip — no re-render through a different code path. The silent reload afterwards is reconciliation only.

## Agent Tools

The agent has 4 manual `rig::tool::Tool` implementations in `crates/insight-core/src/tools/` to search and read documents in the conversation's collection. It iteratively gathers evidence to answer user questions, citing sources. There is no direct user-facing search — all document retrieval happens through the agent:

- **search** — keyword search (BM25 via milli), over the conversation's collection
- **read_chunk** — read an indexed text chunk by document + chunk id; refuses chunks outside the conversation's collection
- **list_documents** — flat list of the documents in the collection, with no per-collection grouping
- **get_collection_terms** — frequent/representative terms of the collection

A search hit carries the passage and the document/chunk identity; it has no collection id or name. Resolving a passage to source units is not the search tool's job; that belongs to the citation layer. Page counts in tool output are optional because the source may have no recorded unit structure.

Tool outputs are typed serializable structs and errors are `ToolExecutionError` with precise kinds (`not_found`, …) — rig converts them into structured model output at dispatch time.

## Data Model

### Conversation = One Collection

A conversation belongs to exactly one evidence collection, fixed at creation. There is no multi-collection scope and no mid-conversation scope change: a different collection means a different conversation. Conversations are local to the device and do not sync — sharing a collection shares its documents, not its chats. Deleting a collection deletes its conversations.

### Collection = Namespace

Each collection is an iroh-docs namespace. Sharing a collection = sharing namespace access.

```
Namespace: 7f3a8b2c... ("Climate Research")
│
├── files/abc123/meta                       → document metadata (JSON)
├── files/abc123/source                     → original file bytes (PDF, etc.)
├── files/abc123/extractions/mupdf-pdf-text@1 → extraction record (JSON)
├── files/def456/meta                       → document metadata (JSON)
├── files/def456/source                     → original file bytes
├── files/def456/extractions/mupdf-pdf-text@1 → extraction record (JSON)
├── _hash_index/{hash}                      → duplicate detection index
└── _collection                             → collection settings
```

All document data is grouped under `files/{doc_id}/`. That grouping is our convention, not something iroh prescribes: iroh-docs keys are opaque bytes, and an entry's value is a blob hash. iroh-docs syncs entries and, under its default download policy, fetches the blobs they reference. That is what lets a collection share as a unit.

A source is immutable. Text is a derivation: a document may have many extractions, each produced by a parser and stored under its `{name}@{version}` key. The extraction record points at a text blob and carries the source unit mapping:

```json
{
	"parser": { "name": "mupdf-pdf-text", "version": "1" },
	"text_hash": "...",
	"unit_kind": "page",
	"units": [{ "number": 1, "spans": [{ "start": 0, "end": 1500 }] }]
}
```

`text_hash` is hex because iroh's `Hash` has no serde representation. `units` are the source's addressable positions with the byte spans of the extraction text attributed to each: for a PDF, one unit per page, numbered from 1. A page with no text is a unit with empty `spans`. Units are ordered by number; spans within a unit are ascending and non-overlapping. `unit_kind` is `null` when the parser records no unit structure. Parser names must not contain `/` or `@`; versions are bumped by hand when the parser's output changes.

### Document Metadata (files/{id}/meta)

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

`extraction` names the document's canonical extraction — the one search indexes. It is `null` before any extraction exists, and the document then has no text and an unknown page count. The page count is derived from the canonical extraction's units, never stored alongside them. There is no migration for documents that predate extractions: they simply have no canonical extraction until re-imported.

### What Syncs vs What's Local

| Data            | Syncs                       | Stored in  |
| --------------- | --------------------------- | ---------- |
| PDF files       | Yes                         | iroh-blobs |
| Extractions     | Yes                         | iroh-blobs |
| File metadata   | Yes                         | iroh-docs  |
| Collection info | Yes                         | iroh-docs  |
| Search index    | No (built from synced text) | milli      |
| Conversations   | No                          | local disk |

Search is local keyword indexing over the canonical extraction's text: each peer builds its own milli index from the synced extraction text. Nothing beyond the raw document data is derived or replicated.

## Document Ingestion

### Local Import

1. User adds PDF to collection
2. **Store**: original PDF bytes written to `files/{id}/source` (iroh), with `files/{id}/meta` and the `_hash_index` duplicate check
3. **Extract**: MuPDF produces an extraction (digital text only; one unit per page with its text spans) written to `files/{id}/extractions/{parser_key}`. The first extraction becomes the document's canonical extraction
4. **Index**: the watcher triggers the index worker, which chunks the canonical extraction's text with a character-based splitter (~1500 chars, ~200 overlap) and indexes chunks in milli for full-text search. Each chunk stores its byte span in the extraction text; page references are not stored on the chunk

Selecting a different extraction as canonical changes what search indexes; the extraction entries themselves never change once written. A source is never modified — changed content is a new document.

**No OCR**: text extraction uses MuPDF's text layer only (its OCR device is not built), so scanned PDFs (images of text) import fine but yield little or no extracted text and are not text-searchable. This is a known limitation.

### On Sync

When document entries arrive from a peer, iroh-docs automatically syncs the entry content blobs. The watcher reacts to `files/*/meta` and extraction entries (local or remote) by queueing indexing for the document:

1. The index worker reads the canonical extraction and its text blob
2. Chunks of that text are indexed in milli for search
3. If the extraction entry arrived before its content, the watcher records the content hash and queues indexing when the matching `ContentReady` event arrives. The index worker skips cleanly while the extraction or text is not yet available
4. Source file at `files/{id}/source` is available immediately

## Sync Model

Collections sync fully between peers — all documents are copied so you have them even when colleagues are offline.

## Settings

Settings v2 (`~/.local/share/insight/settings.json`):

```json
{
	"provider": {
		"family": "openai/openai",
		"model": "gpt-4o-mini",
		"api_key": "...",
		"base_url": "..."
	}
}
```

`provider` is `null` when chat is unconfigured. `family` is a rig provider selection id; `api_key` is required only by selections rig says need a credential, and `base_url` overrides the selection's endpoint. The configured provider is the only place a key is stored. There is intentionally no migration of older settings files — no users yet.

## Development

Requires libclang for `mupdf-sys`'s bindgen step: `pacman -S clang` on Arch,
`apt install libclang-dev clang` on Debian/Ubuntu, Xcode command line tools on
macOS, LLVM (`LIBCLANG_PATH`) on Windows.

```bash
pnpm install
pnpm tauri dev    # Development mode
pnpm tauri build  # Release build
```

## Understanding Dependencies

Prefer local tools over web searches for understanding Rust dependencies:

- `cargo doc --open` - Generate and browse docs for exact dependency versions
- `cargo tree` - View dependency graph and enabled features
- Source at `~/.cargo/registry/src/` or `~/.cargo/git/checkouts/`
- For rig specifically: the local checkout at `~/Projects/rig` (tag `v0.46.0`, matching the crates.io dependency) is the reference — read `~/Projects/rig/AGENTS.md` and the examples under `~/Projects/rig/examples/` before writing code that touches rig APIs

This ensures version accuracy and works offline.

## Testing

Test behaviour, not implementation. Assert what a caller observes — return
values, files written, events emitted, what the user sees — through public
interfaces; never private helpers, internal state, or call order.

- A refactor that changes no behaviour changes no test. If it does, the test
  was pinning implementation.
- Mock at the boundary, never our own code: rig's test-utils model for
  inference, temp directories for iroh/milli, `mockIPC` for Tauri commands.
- Prefer integration: drive a module the way its real caller does. Unit tests
  are for pure logic — the transcript fold, chunking, offset-to-unit mapping.
- No test-only APIs or hooks in production code. No smoke tests.
- A test must be able to fail: assert an outcome, not that a fake was called or
  a line was logged.

### Backend (Rust)

```bash
cd src-tauri && cargo test
```

Use `#[tokio::test]` for async tests; give each test its own temporary data
directory.

### Frontend (Svelte)

```bash
pnpm test        # Watch mode
pnpm test:run    # Single run (CI)
```

Stack: Vitest + @testing-library/svelte + jsdom

- Tests co-located with components: `Component.test.ts`
- SvelteKit mocks in `src/tests/mocks/` (`$app/paths`, `$app/environment`)
- Tauri API mocking via `@tauri-apps/api/mocks`

```ts
import { mockIPC } from '@tauri-apps/api/mocks';

mockIPC((cmd, args) => {
	if (cmd === 'list_providers') {
		return ['openai/openai', 'ollama/openai'];
	}
});
```

## Data Storage

```
~/.local/share/insight/
├── iroh/                          # iroh data (blobs, docs)
├── search/                        # milli index
├── settings.json                  # settings v2 (chat provider)
└── conversations/
    └── {collection_id}/
        └── {conversation_id}/
            ├── logs/{conversation_id}.jsonl   # rig session log
            └── files/meta.json                # rig SessionMeta (title, updated ms)
```

Collection ids are canonical `iroh_docs::NamespaceId` strings, so the directory names cannot escape the conversations root. Each conversation is one rig `DirStore`; `SessionMeta` is the only conversation metadata, with no separate `metadata/*.json` sidecar and no `logs/` at the root. There is no migration from earlier conversation layouts; they are not read.

## Conventions

- Prefer Rust stdlib where possible
- `tokio` runtime (required by iroh)
- Svelte 5 runes, TypeScript
- Agent events streamed via Tauri emit system
- Error types via `thiserror` — no `String` errors
- Doc comments on public items
- **Rig conventions**: for code touching rig APIs, follow `~/Projects/rig/AGENTS.md` and the examples under `~/Projects/rig/examples/` as the style reference
