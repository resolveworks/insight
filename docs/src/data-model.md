# Data Model

## Collections as Namespaces

Each collection is an iroh-docs namespace. Sharing a collection means sharing namespace access.

```
Namespace: 7f3a8b2c... ("Climate Research")
│
├── files/abc123     → blob with metadata JSON
├── files/def456     → blob with metadata JSON
└── _collection      → blob with collection settings
```

## Document Metadata

Document metadata is stored as a blob and referenced by an entry in the namespace:

```json
{
	"name": "paper.pdf",
	"pdf_hash": "blake3-hash-of-pdf",
	"text_hash": "blake3-hash-of-extracted-text",
	"tags": ["research", "climate"],
	"created_at": "2024-01-15T10:30:00Z"
}
```

## Content-Addressed Storage

All file content (PDFs, extracted text) is stored in iroh-blobs using content-addressing:

- Files are identified by their BLAKE3 hash
- Duplicate files are automatically deduplicated
- Content can be verified for integrity

## Embedding Sync

Embeddings are stored in iroh-docs under `embeddings/{doc_id}/{model_id}`. This design:

- **Avoids redundant computation** — generating embeddings is expensive, so peers share them
- **Preserves model flexibility** — different peers can use different embedding models
- **Enables offline use** — embeddings sync with documents, ready for immediate use

When a peer receives a document, it checks for existing embeddings matching its configured model. If found, they're used directly. If not (different model or new document), embeddings are generated locally and stored for other peers to use.

## Sync Model

Collections sync fully between peers—all documents are copied locally. This ensures you have access to everything even when colleagues are offline.

## Local Storage Locations

```
~/.local/share/insight/
├── iroh/               # iroh data (blobs, docs)
└── search/             # milli index

~/.cache/huggingface/hub/
└── models--*/          # Downloaded models (LLM + embedding)
```

On Windows, app data is under `%LOCALAPPDATA%\insight\` and models under `%USERPROFILE%\.cache\huggingface\hub\`.
