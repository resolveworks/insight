# Insight - Local-First Research Agent

A local-first research agent for evidence-based journalism. Think Claude Code, but for documents and investigations.

## Vision

Newsrooms have documents but no good way to search, analyze, and share them without relying on cloud services they don't trust. Insight keeps document storage, search, and sync entirely local — while chat runs on user-configured remote providers (OpenAI, Anthropic). Only the conversation goes to the provider; documents never leave your machine.

## Core Stack

| Component       | Library                 | Purpose                                            |
| --------------- | ----------------------- | -------------------------------------------------- |
| App framework   | Tauri 2.0               | Desktop app (Rust backend, web frontend)           |
| UI              | Svelte 5                | Frontend                                           |
| Styling         | Tailwind 4              | Utility-first CSS (no theme() in component styles) |
| Chat inference  | OpenAI / Anthropic APIs | Remote LLM providers, configured in Settings       |
| P2P / Sync      | iroh                    | Connections, NAT traversal, sync                   |
| Content storage | iroh-blobs              | Content-addressed file storage                     |
| Metadata sync   | iroh-docs               | CRDT key-value store for metadata                  |
| Real-time       | iroh-gossip             | Pub/sub for live updates                           |
| Search          | milli                   | Full-text keyword search (BM25)                    |
| PDF text        | lopdf                   | Digital text extraction                            |

## Architecture

### Agent Architecture

```
User Query
    ↓
Remote LLM (OpenAI / Anthropic)
    ↓
Tool Calling Loop
    ↓
Synthesized Answer (with citations)
```

Inference is remote-only: the chat provider is configured in Settings (API key + model). Documents never leave the machine — only the conversation (queries and retrieved excerpts) is sent to the configured provider.

## Agent Tools

The agent has tools to search and read documents from indexed collections. It iteratively gathers evidence to answer user questions, citing sources. There is no direct user-facing search—all document retrieval happens through the agent. Tools include search (keyword, BM25 via milli), reading chunks, listing documents, and collection terms.

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

Collections sync fully between peers—all documents are copied so you have them even when colleagues are offline.

## Settings

`~/.local/share/insight/settings.json` holds chat provider configuration: `provider` (openai/anthropic config), `openai_api_key`, `anthropic_api_key`. Legacy local-model settings are dropped gracefully on upgrade.

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
├── iroh/               # iroh data (blobs, docs)
├── search/             # milli index
└── conversations/      # chat history
```

## Conventions

- Prefer Rust stdlib where possible
- `tokio` runtime (required by iroh)
- Svelte 5 runes, TypeScript
- Agent events streamed via Tauri emit system
