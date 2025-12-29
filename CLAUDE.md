# Insight - Local-First Research Agent

A local-first research agent for evidence-based journalism. Think Claude Code, but for documents and investigations.

## Vision

Newsrooms have documents but no good way to search, analyze, and share them without relying on cloud services they don't trust. Insight combines local LLM inference with P2P document sync to give journalists an AI research assistant that runs entirely on their hardware.

## Core Stack

| Component       | Library     | Purpose                                                   |
| --------------- | ----------- | --------------------------------------------------------- |
| App framework   | Tauri 2.0   | Desktop app (Rust backend, web frontend)                  |
| UI              | Svelte 5    | Frontend                                                  |
| Styling         | Tailwind 4  | Utility-first CSS (no theme() in component styles)        |
| LLM inference   | mistralrs   | Local model loading and inference (GGUF format)           |
| Model download  | hf-hub      | Fetch models from HuggingFace                             |
| P2P / Sync      | iroh        | Connections, NAT traversal, sync                          |
| Content storage | iroh-blobs  | Content-addressed file storage                            |
| Metadata sync   | iroh-docs   | CRDT key-value store for metadata                         |
| Real-time       | iroh-gossip | Pub/sub for live updates                                  |
| Search          | milli       | Full-text + vector hybrid search (uses candle internally) |
| PDF processing  | lopdf       | Text extraction                                           |

## Architecture

### Agent Architecture

```
User Query
    ↓
Local LLM (via mistralrs)
    ↓
Tool Calling Loop
    ↓
Synthesized Answer (with citations)
```

With local models, the agent runs entirely on-device—no data leaves your machine.

## Models

Insight is batteries-included: local models are downloaded from HuggingFace and run on-device via mistralrs. Remote model providers (OpenAI, Anthropic, etc.) will also be supported for users who prefer them.

### Local Models

| Model           | Size   | Notes                  |
| --------------- | ------ | ---------------------- |
| Qwen3 8B Q4_K_M | 5 GB   | Default, good balance  |
| Qwen3 4B Q4_K_M | 2.5 GB | Lightweight, faster    |
| Qwen3 8B Q8_0   | 8.5 GB | Higher quality, slower |

GPU acceleration available via feature flags:

- `cuda` - NVIDIA GPUs
- `metal` - Apple Silicon
- `flash-attn` - Flash attention optimization

## Agent Tools

The agent has tools to search and read documents from indexed collections. It iteratively gathers evidence to answer user questions, citing sources. There is no direct user-facing search—all document retrieval happens through the agent.

## Data Model

### Collection = Namespace

Each collection is an iroh-docs namespace. Sharing a collection = sharing namespace access.

```
Namespace: 7f3a8b2c... ("Climate Research")
│
├── files/abc123/meta                → document metadata (JSON)
├── files/abc123/text                → extracted text content
├── files/abc123/source              → original file bytes (PDF, etc.)
├── files/abc123/embeddings/qwen3    → chunked text + vectors for model
├── files/def456/meta                → document metadata (JSON)
├── files/def456/text                → extracted text content
├── files/def456/source              → original file bytes
├── files/def456/embeddings/qwen3    → chunked text + vectors for model
├── _hash_index/{hash}               → duplicate detection index
└── _collection                      → collection settings
```

All document data (content, source, embeddings) is grouped under `files/{doc_id}/`. This is the idiomatic iroh pattern: each entry's content IS the blob data. When iroh-docs syncs entries, it automatically syncs their content blobs, enabling seamless P2P document sharing.

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

| Data            | Syncs                       | Stored in                |
| --------------- | --------------------------- | ------------------------ |
| PDF files       | Yes                         | iroh-blobs               |
| Extracted text  | Yes                         | iroh-blobs               |
| File metadata   | Yes                         | iroh-docs                |
| Collection info | Yes                         | iroh-docs                |
| Embeddings      | Yes (keyed by model)        | iroh-docs                |
| Search index    | No (built from synced data) | milli                    |
| LLM models      | No                          | ~/.cache/huggingface/hub |

### Embedding Sync Strategy

Embeddings are stored under `files/{doc_id}/embeddings/{model_id}`, grouping all document data together. Each embedding entry contains both the chunked text and vectors:

```json
{
	"model_id": "qwen3",
	"dimensions": 1024,
	"chunks": [
		{
			"index": 0,
			"content": "text of chunk...",
			"vector": [0.1, 0.2, ...],
			"start_page": 1,
			"end_page": 1
		}
	],
	"created_at": "2024-01-15T10:30:00Z"
}
```

This design:

- **Avoids redundant computation** — generating embeddings is expensive, so peers share them
- **Preserves model flexibility** — different peers can use different embedding models
- **Enables offline use** — embeddings sync with documents, ready for immediate search
- **Groups all document data** — everything about a document lives under one prefix

When a peer receives a document, it checks for existing embeddings matching its configured model. If found, they're used directly. If not (different model or new document), embeddings are generated locally and stored for other peers to use.

## Document Ingestion

### Local Import

1. User adds PDF to collection
2. Extract text via lopdf
3. Store three entries in iroh-docs:
   - `files/{id}/meta` — metadata JSON
   - `files/{id}/text` — extracted text
   - `files/{id}/source` — original PDF bytes
4. Chunk text, generate embeddings, store at `files/{id}/embeddings/{model_id}`
5. Index chunks in milli for search

### On Sync

When document entries arrive from a peer, iroh-docs automatically syncs the entry content blobs. The SyncWatcher listens for `files/*/meta` entries and triggers processing:

1. Text is already available at `files/{id}/text` (synced by iroh)
2. Check for existing embeddings at `files/{id}/embeddings/{model_id}`
3. If embeddings exist for configured model, use them; otherwise generate locally
4. Index chunks in milli for search
5. Source file at `files/{id}/source` is available immediately

## Sync Model

Collections sync fully between peers—all documents are copied so you have them even when colleagues are offline.

## Development

```bash
pnpm install
pnpm tauri dev    # Development mode
pnpm tauri build  # Release build
```

### GPU Builds

```bash
# NVIDIA
cargo build --release --features "cuda flash-attn cudnn"

# Apple Silicon
cargo build --release --features metal
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

- Critical user flows (document ingestion, chat)
- Complex logic that's hard to verify manually
- Bug fixes (prevent regressions)

### Guidelines

- Test behavior, not implementation details
- Use `#[tokio::test]` for async Rust tests
- Temporary directories for test data (avoid polluting real data)
- Mock iroh/milli only when necessary for isolation

## Data Storage

```
~/.local/share/insight/
├── iroh/               # iroh data (blobs, docs)
└── search/             # milli index

~/.cache/insight/
└── models/             # Downloaded LLM files (GGUF + tokenizers)
```

## Conventions

- Prefer Rust stdlib where possible
- `tokio` runtime (required by iroh)
- Svelte 5 runes, TypeScript
- Agent events streamed via Tauri emit system
