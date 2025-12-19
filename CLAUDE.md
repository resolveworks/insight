# Insight - Local-First Document Search

A local-first, P2P document management and search application.

## Vision

A local-first document search tool for journalists. The problem: newsrooms have documents but no good way to search and share them without relying on cloud services they don't trust.

## Core Stack

| Component           | Library     | Purpose                                                   |
| ------------------- | ----------- | --------------------------------------------------------- |
| App framework       | Tauri 2.0   | Desktop app (Rust backend, web frontend)                  |
| UI                  | Svelte 5    | Frontend                                                  |
| P2P / Sync          | iroh        | Connections, NAT traversal, sync                          |
| Content storage     | iroh-blobs  | Content-addressed file storage                            |
| Metadata sync       | iroh-docs   | CRDT key-value store for metadata                         |
| Real-time           | iroh-gossip | Pub/sub for live updates                                  |
| Search + embeddings | milli       | Full-text + vector hybrid search (uses candle internally) |
| PDF processing      | lopdf       | Text extraction (no built-in viewer, open in system apps) |

## Architecture

One binary, two modes—Core (iroh + milli) is shared:

```
insight              → GUI mode (Tauri + Svelte)
insight --headless   → Server mode (no UI)
```

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

| Data            | Syncs        | Stored in  |
| --------------- | ------------ | ---------- |
| PDF files       | Yes          | iroh-blobs |
| Extracted text  | Yes          | iroh-blobs |
| File metadata   | Yes          | iroh-docs  |
| Collection info | Yes          | iroh-docs  |
| Embeddings      | No (derived) | milli      |
| Search index    | No (derived) | milli      |

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

The "server" is just the same app running in headless mode with better uptime.

## Search

Each node builds its own milli index from synced data:

1. Receive synced metadata (includes extracted text hash)
2. Fetch extracted text blob
3. Index in milli (embeddings generated internally via candle)

## Development

```bash
pnpm install
pnpm tauri dev          # GUI mode
cargo run -- --headless # Headless mode (from src-tauri/)
pnpm tauri build        # Release build
```

## Testing

Minimal strategy focused on Rust backend where critical logic lives.

```bash
cd src-tauri && cargo test
```

### What to Test

1. **Core modules** (unit tests) - Storage, sync, search operations
2. **Tauri commands** (integration tests) - The frontend ↔ backend bridge
3. **Critical path** - Collection CRUD, document ingestion, search queries

### Guidelines

- Test behavior, not implementation details
- Use `#[tokio::test]` for async tests
- Temporary directories for test data (avoid polluting real data)
- Mock iroh/milli only when necessary for isolation

## Data Storage

```
~/.local/share/insight/
├── iroh/               # iroh data (blobs, docs)
└── search/             # milli index
```

## Conventions

- Prefer Rust stdlib where possible
- `tokio` runtime (required by iroh)
- Svelte 5 runes, TypeScript
