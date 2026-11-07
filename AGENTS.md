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
| PDF text        | lopdf       | Digital text extraction                                                       |

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
- **`runtime.rs`** — `send_message` builds a fresh rig agent **per request**: a stable preamble (`BASE_SYSTEM_PROMPT`) + the 4 document tools (manual `rig::tool::Tool` implementations) + `FileConversationMemory` + `default_max_turns(100)`. Each tool owns a clone of the per-run `ToolState` (index, storage, active collections); Insight keeps these live handles on the tool instances rather than in `ToolContext`. The conversation's active collections are attached as a run document with `AgentRunner::document` on every turn — non-sticky, so mid-conversation scope changes take effect on the next model call without touching stored history. The multi-turn stream is forwarded to the frontend as JSON `MultiTurnStreamItem` events on `agent-event-{conversation_id}`; cancellation drops the stream via a `CancellationToken`. Stream failures return as `Err`; the Tauri command emits the single terminal `streamError`. `generate_title` and `predict_next_message` are bare-model completions (no tools, no agent loop) over the same provider settings.
- **`memory.rs`** — `FileConversationMemory` wraps rig_memory's directory-backed `DirStore` for conversation logs and a rig `DirFiles` namespace for metadata; it does not implement a `ConversationMemory` trait. A `ConversationMeta` sidecar holds app-facing data (title, linked collections, timestamps).
- **`tools/`** — manual `rig::tool::Tool` implementations over milli search + iroh storage; each owns a clone of the per-run `ToolState` (index, storage, collection scope).

Inference is remote-only: the chat provider is configured in Settings. Documents never leave the machine — only the conversation (queries and retrieved excerpts) is sent to the configured provider.

### Event Flow

`send_message` streams rig `MultiTurnStreamItem`s; the Tauri command layer (`src-tauri/src/commands/conversations.rs`) serializes them and emits them on `agent-event-{id}`. When the run fails, the runtime returns `Err` and the command emits the single terminal `{"type":"streamError","message"}` item.

The frontend renders ONE view model for both stored history and the live run. A pure fold (`src/lib/chat/transcript.ts`, `applyItem`) accumulates the stream into a `LiveRun` shaped like the stored transcript (chronological segments; a tool result closes the current assistant turn), and `buildTranscript` normalizes stored + live into a flat `TranscriptItem[]` (the conversations store exposes it as `getTranscript()`). Live-only state is data on the items (`provisional`, `executing`, `streaming` flags), so `Chat.svelte` has a single render path: a run committing swaps the provisional tail for the `finalResponse` payload's `messages` synchronously (tracked by the store's `runBase`) and the flags flip — no re-render through a different code path. The silent reload afterwards is reconciliation only.

## Agent Tools

The agent has 4 manual `rig::tool::Tool` implementations in `crates/insight-core/src/tools/` to search and read documents from indexed collections. It iteratively gathers evidence to answer user questions, citing sources. There is no direct user-facing search — all document retrieval happens through the agent:

- **search** — keyword search (BM25 via milli), scoped to active collections
- **read_chunk** — read an indexed text chunk by document + chunk id
- **list_documents** — list documents in the active collections
- **get_collection_terms** — frequent/representative terms of a collection

Tool outputs are typed serializable structs and errors are `ToolExecutionError` with precise kinds (`not_found`, …) — rig converts them into structured model output at dispatch time.

## Data Model

### Collection = Namespace

Each collection is an iroh-docs namespace. Sharing a collection = sharing namespace access.

```
Namespace: 7f3a8b2c... ("Climate Research")
│
├── files/abc123/meta                → document metadata (JSON)
├── files/abc123/text                → extracted text content
├── files/abc123/source              → original file bytes (PDF, etc.)
├── files/def456/meta                → document metadata (JSON)
├── files/def456/text                → extracted text content
├── files/def456/source              → original file bytes
├── _hash_index/{hash}               → duplicate detection index
└── _collection                      → collection settings
```

All document data is grouped under `files/{doc_id}/`. This is the idiomatic iroh pattern: each entry's content IS the blob data. When iroh-docs syncs entries, it automatically syncs their content blobs, enabling seamless P2P document sharing.

### Document Metadata (files/{id}/meta)

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

### What Syncs vs What's Local

| Data            | Syncs                       | Stored in  |
| --------------- | --------------------------- | ---------- |
| PDF files       | Yes                         | iroh-blobs |
| Extracted text  | Yes                         | iroh-blobs |
| File metadata   | Yes                         | iroh-docs  |
| Collection info | Yes                         | iroh-docs  |
| Search index    | No (built from synced text) | milli      |

Search is local keyword indexing over synced text: each peer builds its own milli index from the documents' text. Nothing beyond the raw document data is derived or replicated.

## Document Ingestion

### Local Import

1. User adds PDF to collection
2. **Store**: original PDF bytes written to `files/{id}/source` (iroh)
3. **Extract**: per-page text extracted with lopdf (digital text only; page boundaries recorded), written to `files/{id}/text`, alongside `files/{id}/meta`
4. **Index**: the watcher triggers the index worker, which chunks text with a character-based splitter (~1500 chars, ~200 overlap, page-mapped) and indexes chunks in milli for full-text search

**No OCR**: text extraction uses lopdf only, so scanned PDFs (images of text) import fine but yield little or no extracted text and are not text-searchable. This is a known limitation.

### On Sync

When document entries arrive from a peer, iroh-docs automatically syncs the entry content blobs. The SyncWatcher listens for `files/*/text` entries and triggers indexing:

1. Text is already available at `files/{id}/text` (synced by iroh)
2. Chunks are indexed in milli for search
3. Source file at `files/{id}/source` is available immediately

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
  are for pure logic — the transcript fold, chunking, page boundaries.
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
├── settings.json                  # settings v2 (provider + API keys)
└── conversations/
    ├── logs/
    │   └── {conversation_id}.jsonl   # rig conversation history
    └── metadata/
        └── {conversation_id}.json    # title, collections, timestamps
```

The old `conversations/{id}/messages.json` + `meta.json` layout is rejected, not migrated. Both stores write atomically: a crash mid-write leaves the previous file intact.

## Conventions

- Prefer Rust stdlib where possible
- `tokio` runtime (required by iroh)
- Svelte 5 runes, TypeScript
- Agent events streamed via Tauri emit system
- Error types via `thiserror` — no `String` errors
- Doc comments on public items
- **Rig conventions**: for code touching rig APIs, follow `~/Projects/rig/AGENTS.md` and the examples under `~/Projects/rig/examples/` as the style reference
