<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";

  let searchQuery = $state("");
  let results = $state<any[]>([]);
  let collections = $state<any[]>([]);
  let documents = $state<any[]>([]);
  let importing = $state(false);
  let newCollectionName = $state("");

  async function search() {
    if (!searchQuery.trim()) {
      results = [];
      return;
    }
    try {
      results = await invoke("search", { query: searchQuery });
    } catch (e) {
      console.error("Search failed:", e);
    }
  }

  async function importPdf() {
    const files = await open({
      multiple: true,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });

    if (!files) return;

    importing = true;
    const paths = Array.isArray(files) ? files : [files];

    for (const path of paths) {
      try {
        const doc = await invoke("import_pdf", { path, collectionId: null });
        documents = [...documents, doc];
      } catch (e) {
        console.error("Failed to import:", path, e);
      }
    }
    importing = false;
  }

  async function createCollection() {
    if (!newCollectionName.trim()) return;
    try {
      const collection = await invoke("create_collection", { name: newCollectionName });
      collections = [...collections, collection];
      newCollectionName = "";
    } catch (e) {
      console.error("Failed to create collection:", e);
    }
  }

  async function loadCollections() {
    try {
      collections = await invoke("get_collections");
    } catch (e) {
      console.error("Failed to load collections:", e);
    }
  }

  $effect(() => {
    loadCollections();
  });
</script>

<main>
  <header>
    <h1>Insight</h1>
    <p class="subtitle">Local-first document search</p>
  </header>

  <div class="search-container">
    <input
      type="text"
      placeholder="Search documents..."
      bind:value={searchQuery}
      onkeydown={(e) => e.key === "Enter" && search()}
    />
    <button onclick={search}>Search</button>
    <button onclick={importPdf} disabled={importing}>
      {importing ? "Importing..." : "Import PDF"}
    </button>
  </div>

  <div class="content">
    <aside class="sidebar">
      <h2>Collections</h2>
      <div class="new-collection">
        <input
          type="text"
          placeholder="New collection..."
          bind:value={newCollectionName}
          onkeydown={(e) => e.key === "Enter" && createCollection()}
        />
        <button onclick={createCollection}>+</button>
      </div>
      {#if collections.length === 0}
        <p class="empty">No collections yet</p>
      {:else}
        <ul>
          {#each collections as collection}
            <li>{collection.name}</li>
          {/each}
        </ul>
      {/if}

      <h2>Imported Documents</h2>
      {#if documents.length === 0}
        <p class="empty">No documents imported</p>
      {:else}
        <ul>
          {#each documents as doc}
            <li title={`${doc.page_count} pages`}>{doc.name}</li>
          {/each}
        </ul>
      {/if}
    </aside>

    <section class="results">
      {#if results.length === 0}
        <p class="empty">
          {searchQuery ? "No results found" : "Enter a search query to find documents"}
        </p>
      {:else}
        <ul>
          {#each results as result}
            <li class="result-item">
              <h3>{result.document.name}</h3>
              <p class="snippet">{result.snippet}</p>
              <span class="score">Score: {result.score.toFixed(2)}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  </div>
</main>

<style>
  :global(body) {
    margin: 0;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    background: #1a1a2e;
    color: #eee;
  }

  main {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
  }

  header {
    padding: 1.5rem 2rem;
    background: #16213e;
    border-bottom: 1px solid #0f3460;
  }

  h1 {
    margin: 0;
    font-size: 1.5rem;
    color: #e94560;
  }

  .subtitle {
    margin: 0.25rem 0 0;
    font-size: 0.875rem;
    color: #888;
  }

  .search-container {
    display: flex;
    gap: 0.5rem;
    padding: 1rem 2rem;
    background: #16213e;
  }

  input {
    flex: 1;
    padding: 0.75rem 1rem;
    border: 1px solid #0f3460;
    border-radius: 6px;
    background: #1a1a2e;
    color: #eee;
    font-size: 1rem;
  }

  input:focus {
    outline: none;
    border-color: #e94560;
  }

  button {
    padding: 0.75rem 1.5rem;
    border: none;
    border-radius: 6px;
    background: #e94560;
    color: white;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.2s;
  }

  button:hover:not(:disabled) {
    background: #d63850;
  }

  button:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .content {
    display: flex;
    flex: 1;
  }

  .sidebar {
    width: 280px;
    padding: 1rem;
    background: #16213e;
    border-right: 1px solid #0f3460;
    overflow-y: auto;
  }

  .sidebar h2 {
    margin: 1rem 0 0.5rem;
    font-size: 0.75rem;
    text-transform: uppercase;
    color: #888;
  }

  .sidebar h2:first-child {
    margin-top: 0;
  }

  .new-collection {
    display: flex;
    gap: 0.5rem;
    margin-bottom: 0.5rem;
  }

  .new-collection input {
    flex: 1;
    padding: 0.5rem;
    font-size: 0.875rem;
  }

  .new-collection button {
    padding: 0.5rem 0.75rem;
    font-size: 1rem;
  }

  .sidebar ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .sidebar li {
    padding: 0.5rem;
    border-radius: 4px;
    cursor: pointer;
    font-size: 0.875rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .sidebar li:hover {
    background: #0f3460;
  }

  .results {
    flex: 1;
    padding: 1.5rem 2rem;
    overflow-y: auto;
  }

  .results ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .result-item {
    padding: 1rem;
    margin-bottom: 1rem;
    background: #16213e;
    border-radius: 8px;
    border: 1px solid #0f3460;
  }

  .result-item h3 {
    margin: 0 0 0.5rem;
    font-size: 1rem;
    color: #e94560;
  }

  .snippet {
    margin: 0;
    font-size: 0.875rem;
    color: #aaa;
    line-height: 1.5;
  }

  .score {
    display: inline-block;
    margin-top: 0.5rem;
    font-size: 0.75rem;
    color: #666;
  }

  .empty {
    color: #666;
    font-style: italic;
    font-size: 0.875rem;
  }
</style>
