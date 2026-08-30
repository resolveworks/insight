# Insight - Local-First Research Agent

A local-first research agent for evidence-based journalism. Think Claude Code, but for documents and investigations.

## Vision

Newsrooms have documents but no good way to search, analyze, and share them without relying on cloud services they don't trust. Insight keeps document storage, search, and sync entirely local — while chat runs on a user-configured remote LLM provider (25+ families via the rig registry: OpenAI, Anthropic, Gemini, Groq, local servers like Ollama/llama.cpp, ...). Only the conversation goes to the provider; documents never leave your machine.

## Core Stack

| Component       | Library            | Purpose                                                       |
| --------------- | ------------------ | ------------------------------------------------------------- |
| App framework   | Tauri 2.0          | Desktop app (Rust backend, web frontend)                      |
| UI              | Svelte 5           | Frontend                                                      |
| Styling         | Tailwind 4         | Utility-first CSS (no theme() in component styles)            |
| Chat inference  | [rig]              | LLM framework: 25 provider families via a registry, agent runtime, tools, memory |
| P2P / Sync      | iroh               | Connections, NAT traversal, sync                              |
| Content storage | iroh-blobs         | Content-addressed file storage                                |
| Metadata sync   | iroh-docs          | CRDT key-value store for metadata                             |
| Real-time       | iroh-gossip        | Pub/sub for live updates                                      |
| Search          | milli              | Full-text keyword search (BM25)                               |
| PDF text        | lopdf              | Digital text extraction                                       |

[rig]: https://github.com/0xPlaygrounds/rig

The rig dependency is pinned to git rev `475cfddc` (v0.42.0 + 30 commits). A local reference checkout lives at `../rig` — use it as the API reference for all rig types; see `../rig/AGENTS.md` and its examples for style.

## Architecture

### Agent Architecture

```
User Query
    ↓
rig Agent (per request: preamble + tools + memory + max turns)
    ↓
Registry-built Model (any of ~25 rig provider families)
    ↓
Tool Calling Loop (multi-turn)
    ↓
Synthesized Answer (with citations)
```

Key pieces, all in `crates/insight-core/`:

- **`providers/`** — a data-driven registry over all rig chat-completion provider families: `list_families()` (UI descriptors; local-server families like ollama/llamacpp need no API key), `build_model()` (validates settings and returns a rig `ModelHandle`), and `list_models()` (rig's `ModelListingClient` capability for families that support remote model listing).
- **`runtime.rs`** — `send_message` builds a fresh rig agent **per request**: a stable preamble (`BASE_SYSTEM_PROMPT`) + the 4 document tools + `FileConversationMemory` + `default_max_turns(100)`. A scope hook (`AgentHook`) injects the conversation's active collections as an `extra_context` document (`RequestPatch`) on every turn — non-sticky, so mid-conversation scope changes take effect on the next model call without touching stored history. The multi-turn stream is forwarded to the frontend as JSON `MultiTurnStreamItem` events on `agent-event-{conversation_id}`; cancellation drops the stream via a `CancellationToken`. `generate_title` and `predict_next_message` are bare-model completions (no tools, no agent loop) over the same provider settings.
- **`memory.rs`** — `FileConversationMemory` implements rig's `ConversationMemory` trait on a directory tree; a `ConversationMeta` sidecar holds app-facing data (title, linked collections, timestamps).
- **`tools/`** — typed rig `Tool` implementations over milli search + iroh storage.

Inference is remote-only: the chat provider is configured in Settings. Documents never leave the machine — only the conversation (queries and retrieved excerpts) is sent to the configured provider.

### Event Flow

`send_message` streams rig `MultiTurnStreamItem`s; the Tauri command layer (`src-tauri/src/commands/conversations.rs`) serializes them and emits them on `agent-event-{id}`. The backend appends a terminal `{"type":"streamError","message"}` item when the run fails outside the stream. The frontend mirrors the rig types in `src/lib/types/rig.ts` and renders the live stream.

## Agent Tools

The agent has 4 typed rig tools to search and read documents from indexed collections (`crates/insight-core/src/tools/`). It iteratively gathers evidence to answer user questions, citing sources. There is no direct user-facing search — all document retrieval happens through the agent:

- **search** — keyword search (BM25 via milli), scoped to active collections
- **read_chunk** — read an indexed text chunk by document + chunk id
- **list_documents** — list documents in the active collections
- **get_collection_terms** — frequent/representative terms of a collection

Tool outputs are typed serializable structs — rig converts them into structured model output at dispatch time.

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
  "provider": { "family": "openai", "model": "gpt-4o-mini", "api_key": "...", "base_url": "..." },
  "api_keys": { "openai": "sk-...", "anthropic": "sk-ant-..." }
}
```

`provider` is `null` when chat is unconfigured. `api_keys` stores per-family keys separately from the active provider so switching families keeps previously entered keys. `api_key`/`base_url` are optional per family (local-server families like ollama/llamacpp need neither). There is intentionally no legacy migration — no users yet.

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
- For rig specifically: the pinned checkout at `../rig` (rev `475cfddc`) is the reference — read `../rig/AGENTS.md` and the examples under `../rig/rig-core/examples/` before writing code that touches rig APIs

This ensures version accuracy and works offline.

## Testing

### Backend (Rust)

```bash
cd src-tauri && cargo test
```

Focus on core modules, Tauri commands, and critical paths.

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
	if (cmd === 'search_documents') {
		return [{ title: 'Test Doc' }];
	}
});
```

Focus on:

- Test behavior, not implementation details
- Use `#[tokio::test]` for async Rust tests
- Temporary directories for test data (avoid polluting real data)
- Mock iroh/milli only when necessary for isolation

## Data Storage

```
~/.local/share/insight/
├── iroh/                          # iroh data (blobs, docs)
├── search/                        # milli index
├── settings.json                  # settings v2 (provider + API keys)
└── conversations/
    └── {conversation_id}/
        ├── messages.json          # Vec<rig::completion::Message> — agent history
        └── meta.json              # ConversationMeta sidecar (title, collections, timestamps)
```

All conversation writes are write-temp-then-rename, so a crash mid-write never truncates a stored file.

## Conventions

- Prefer Rust stdlib where possible
- `tokio` runtime (required by iroh)
- Svelte 5 runes, TypeScript
- Agent events streamed via Tauri emit system
- Error types via `thiserror` — no `String` errors
- Doc comments on public items
- **Rig conventions**: for code touching rig APIs, follow `../rig/AGENTS.md` and the examples under `../rig/rig-core/examples/` as the style reference
