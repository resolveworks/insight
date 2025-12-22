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

The agent can search and read documents from the indexed collections. It iteratively gathers evidence to answer questions, citing sources.

## Data Model

### Collection = Namespace

Each collection is an iroh-docs namespace. Sharing a collection = sharing namespace access.

```
Namespace: 7f3a8b2c... ("Climate Research")
│
├── files/abc123     → blob with metadata JSON
├── files/def456     → blob with metadata JSON
└── _collection      → blob with collection settings
```

### Document Metadata (stored as blob, referenced by entry)

```json
{
	"name": "paper.pdf",
	"pdf_hash": "blake3-hash-of-pdf",
	"text_hash": "blake3-hash-of-extracted-text",
	"tags": ["research", "climate"],
	"created_at": "2024-01-15T10:30:00Z"
}
```

### What Syncs vs What's Local

| Data            | Syncs        | Stored in        |
| --------------- | ------------ | ---------------- |
| PDF files       | Yes          | iroh-blobs       |
| Extracted text  | Yes          | iroh-blobs       |
| File metadata   | Yes          | iroh-docs        |
| Collection info | Yes          | iroh-docs        |
| Embeddings      | No (derived) | milli            |
| Search index    | No (derived) | milli            |
| LLM models      | No           | ~/.cache/insight |

## Document Ingestion

### Local Import

1. User adds PDF to collection
2. Extract text via lopdf
3. Store PDF blob → get `pdf_hash`
4. Store text blob → get `text_hash`
5. Create metadata entry in iroh-docs
6. Index text + generate embeddings in milli

### On Sync

When a new metadata entry arrives from a peer:

1. Fetch text blob using `text_hash`
2. Index text + generate embeddings in milli
3. PDF blob fetched on-demand (when user opens document)

## Sync Model

### Peer Collections (colleague's laptop)

Full sync - because they might be offline when you need the files.

### Server Collections (always-on instance)

On-demand - fetch files when needed, server is always available.

The "server" is just the same app running on a machine with better uptime.

## Development

```bash
pnpm install
pnpm tauri dev    # Development mode
pnpm tauri build  # Release build
```

### GPU Builds

```bash
# NVIDIA
cargo build --release --features cuda

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

- Critical user flows (search, document ingestion, chat)
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
