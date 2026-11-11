# Architecture

Insight is a desktop application rather than a hosted document service. Its architecture is designed around two goals: keeping source documents under the user's control and letting each newsroom choose how AI inference and collaboration should work.

## Why a local desktop application?

[Tauri](https://v2.tauri.app/) provides the cross-platform desktop shell, [Svelte](https://svelte.dev/) provides the interface, and Rust handles document processing, storage, search, sync, and chat orchestration. A desktop application can work directly with local files and continue background processing without requiring an application server. Tauri uses the operating system's webview instead of shipping a separate browser runtime.

## How documents become searchable

[MuPDF](https://mupdf.com/) extracts the existing text from PDFs while preserving its page locations. [milli](https://github.com/meilisearch/milli) builds a local keyword search index and ranks matching documents using keyword relevance. Search therefore runs on the user's machine and does not require uploading the collection.

The index holds one record per collection document. Each record contains the document's complete canonical extraction in a single content field, with the document name in a separate searchable field. A search returns ranked documents with a bounded excerpt around the matching text. The agent reads further with `read_document`, which takes the document's extraction key and a character offset into that extraction, keeping excerpts and full text aligned.

milli indexes at most the first 65,536 token positions of a field. A document whose content extends beyond that is shown as partially searchable: its full extraction remains stored and readable, but text past the limit does not appear in search results.

Original documents are immutable, content-addressed blobs. Extracted text is a separate, derived artifact: it can be regenerated or replaced when extraction improves without changing the source document. Content addressing also lets peers verify that they received exactly the data they requested.

## How Insight talks to models

Insight uses [rig](https://github.com/0xPlaygrounds/rig) to provide a consistent agent and tool-calling interface across model providers. When someone asks a question, the agent uses local tools to search the collection and read matching excerpts and surrounding text, then asks the configured model to synthesize an answer from that evidence.

The model provider receives the conversation and retrieved excerpts. It does not receive direct access to the document library or search index. Hosted models and local model servers use the same research workflow, allowing users to choose between managed inference and keeping model processing local.

## How collections sync

Collections use [iroh](https://iroh.computer/) for peer-to-peer replication. Replicated collection metadata records which documents and extractions belong to a collection; content-addressed storage transfers their actual bytes. Direct peer connections and NAT traversal allow colleagues to exchange data without running a central document server.

Recipients get full copies of the documents and extracted text, then build their own search indexes. This makes each copy usable offline and avoids depending on one peer remaining online. Conversations never sync.

A sharing ticket grants access to a collection rather than identifying an account on a central service. That enables direct collaboration, but it also means access cannot be revoked after a ticket is shared.
