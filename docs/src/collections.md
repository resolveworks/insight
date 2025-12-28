# Collections

Collections are how Insight organizes documents. Each collection is a separate namespace with its own documents, search index, and sharing settings.

## Creating Collections

Click **New Collection** and provide a name. Collections can represent:

- A specific investigation or story
- A beat (e.g., "City Council", "Climate")
- A source type (e.g., "Court Documents", "FOIA Responses")

## Adding Documents

Drag and drop PDF files into a collection, or use the **Add Files** button.

Supported formats:

- PDF (primary format)

When you add a document, Insight:

1. Extracts text from the PDF
2. Stores both the original PDF and extracted text
3. Generates embeddings for the AI agent

## Sharing Collections

Collections can be shared peer-to-peer with colleagues.

### Sharing with a Colleague

1. Open the collection settings
2. Click **Share**
3. Send the generated link to your colleague

### Joining a Shared Collection

1. Click **Join Collection**
2. Paste the share link
3. Documents will sync automatically

### Sync Behavior

Collections sync fully—all documents are copied so you have them even when colleagues are offline.

## Collection Storage

Collection data is stored locally:

- **macOS/Linux**: `~/.local/share/insight/`
- **Windows**: `%LOCALAPPDATA%\insight\`
