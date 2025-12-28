# Architecture

Insight is built as a Tauri 2.0 desktop application with a Rust backend and Svelte frontend.

## Core Stack

| Component       | Library     | Purpose                                         |
| --------------- | ----------- | ----------------------------------------------- |
| App framework   | Tauri 2.0   | Desktop app (Rust backend, web frontend)        |
| UI              | Svelte 5    | Frontend                                        |
| Styling         | Tailwind 4  | Utility-first CSS                               |
| LLM inference   | mistralrs   | Local model loading and inference (GGUF format) |
| Model download  | hf-hub      | Fetch models from HuggingFace                   |
| P2P / Sync      | iroh        | Connections, NAT traversal, sync                |
| Content storage | iroh-blobs  | Content-addressed file storage                  |
| Metadata sync   | iroh-docs   | CRDT key-value store for metadata               |
| Real-time       | iroh-gossip | Pub/sub for live updates                        |
| Search          | milli       | Full-text + vector search (used by agent)       |
| PDF processing  | lopdf       | Text extraction                                 |

## Agent Architecture

```
User Query
    ↓
Local LLM (via mistralrs)
    ↓
Tool Calling Loop
    ↓
Synthesized Answer (with citations)
```

The agent has tools for searching and reading documents. It iteratively gathers evidence to answer questions, citing sources along the way. There is no direct user-facing search—all document retrieval happens through the agent.

## Data Flow

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

## What Syncs vs What's Local

| Data            | Syncs                | Stored in                |
| --------------- | -------------------- | ------------------------ |
| PDF files       | Yes                  | iroh-blobs               |
| Extracted text  | Yes                  | iroh-blobs               |
| File metadata   | Yes                  | iroh-docs                |
| Collection info | Yes                  | iroh-docs                |
| Embeddings      | Yes (keyed by model) | iroh-docs                |
| Search index    | No (derived)         | milli (for agent)        |
| LLM models      | No                   | ~/.cache/huggingface/hub |
