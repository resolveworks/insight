<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import Sidebar from "$lib/components/Sidebar.svelte";

  type Tab = "trajectory" | "search" | "files";
  let activeTab = $state<Tab>("search");

  let searchQuery = $state("");
  let results = $state<any[]>([]);
  let collections = $state<any[]>([]);
  let documents = $state<any[]>([]);
  let importing = $state(false);
  let newCollectionName = $state("");
  let selectedCollection = $state<string | null>(null);
  let selectedSearchCollections = $state<Set<string>>(new Set());
  let searching = $state(false);

  // Debounced search-as-you-type
  let searchTimeout: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    const query = searchQuery;
    const filterIds = selectedSearchCollections;

    // Clear previous timeout
    if (searchTimeout) {
      clearTimeout(searchTimeout);
    }

    // Debounce search by 200ms
    searchTimeout = setTimeout(() => {
      performSearch(query, filterIds);
    }, 200);

    return () => {
      if (searchTimeout) clearTimeout(searchTimeout);
    };
  });

  async function performSearch(query: string, filterIds: Set<string>) {
    if (!query.trim()) {
      results = [];
      return;
    }
    searching = true;
    try {
      const collectionIds = filterIds.size > 0 ? Array.from(filterIds) : null;
      results = await invoke("search", { query, collectionIds });
    } catch (e) {
      console.error("Search failed:", e);
    } finally {
      searching = false;
    }
  }

  function toggleSearchCollection(collectionId: string) {
    const newSet = new Set(selectedSearchCollections);
    if (newSet.has(collectionId)) {
      newSet.delete(collectionId);
    } else {
      newSet.add(collectionId);
    }
    selectedSearchCollections = newSet;
  }

  function getCollectionName(collectionId: string): string {
    const col = collections.find((c) => c.id === collectionId);
    return col?.name ?? "Unknown";
  }

  async function importPdf() {
    if (!selectedCollection) {
      console.error("No collection selected");
      return;
    }

    const files = await open({
      multiple: true,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });

    if (!files) return;

    importing = true;
    const paths = Array.isArray(files) ? files : [files];

    for (const path of paths) {
      try {
        const doc = await invoke("import_pdf", { path, collectionId: selectedCollection });
        documents = [...documents, doc];
      } catch (e) {
        console.error("Failed to import:", path, e);
      }
    }
    importing = false;
  }

  async function loadDocuments(collectionId: string) {
    try {
      documents = await invoke("get_documents", { collectionId });
    } catch (e) {
      console.error("Failed to load documents:", e);
      documents = [];
    }
  }

  async function selectCollection(collectionId: string | null) {
    if (selectedCollection === collectionId) {
      selectedCollection = null;
      documents = [];
    } else {
      selectedCollection = collectionId;
      if (collectionId) {
        await loadDocuments(collectionId);
      }
    }
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

  async function deleteCollection(collectionId: string, event: MouseEvent) {
    event.stopPropagation();
    try {
      await invoke("delete_collection", { collectionId });
      collections = collections.filter((c) => c.id !== collectionId);
      if (selectedCollection === collectionId) {
        selectedCollection = null;
        documents = [];
      }
    } catch (e) {
      console.error("Failed to delete collection:", e);
    }
  }

  async function deleteDocument(documentId: string) {
    if (!selectedCollection) return;
    try {
      await invoke("delete_document", { collectionId: selectedCollection, documentId });
      documents = documents.filter((d) => d.id !== documentId);
    } catch (e) {
      console.error("Failed to delete document:", e);
    }
  }

  $effect(() => {
    loadCollections();
  });

  const tabs: { id: Tab; label: string }[] = [
    { id: "trajectory", label: "Trajectory" },
    { id: "search", label: "Search" },
    { id: "files", label: "Files" },
  ];
</script>

<main class="flex h-screen flex-col bg-slate-900 text-slate-100">
  <!-- Tab Navigation -->
  <nav class="flex border-b border-slate-700 bg-slate-800">
    {#each tabs as tab}
      <button
        onclick={() => (activeTab = tab.id)}
        class="px-6 py-3 text-sm font-medium transition-colors {activeTab === tab.id
          ? 'border-b-2 border-rose-500 text-rose-500'
          : 'text-slate-400 hover:text-slate-200'}"
      >
        {tab.label}
      </button>
    {/each}
  </nav>

  <!-- Tab Content -->
  <div class="flex-1 overflow-hidden">
    {#if activeTab === "trajectory"}
      <!-- Trajectory Tab -->
      <div class="flex h-full items-center justify-center p-6 text-center">
        <p class="text-slate-400">Agent chat coming soon.</p>
      </div>
    {:else if activeTab === "search"}
      <!-- Search Tab -->
      <div class="flex h-full">
        <Sidebar title="Filter by Collection">
          {#if collections.length === 0}
            <p class="text-sm italic text-slate-500">No collections</p>
          {:else}
            <ul class="space-y-1">
              {#each collections as collection}
                <li>
                  <label class="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-slate-700">
                    <input
                      type="checkbox"
                      checked={selectedSearchCollections.has(collection.id)}
                      onchange={() => toggleSearchCollection(collection.id)}
                      class="h-4 w-4 rounded border-slate-600 bg-slate-900 text-rose-500 focus:ring-rose-500"
                    />
                    <span class="truncate {selectedSearchCollections.has(collection.id) ? 'text-rose-400' : 'text-slate-300'}">
                      {collection.name}
                    </span>
                  </label>
                </li>
              {/each}
            </ul>
            {#if selectedSearchCollections.size > 0}
              <button
                onclick={() => (selectedSearchCollections = new Set())}
                class="mt-3 text-xs text-slate-500 hover:text-slate-300"
              >
                Clear filters
              </button>
            {/if}
          {/if}
        </Sidebar>

        <!-- Search Content -->
        <div class="flex flex-1 flex-col">
          <div class="flex items-center gap-2 border-b border-slate-700 p-4">
            <input
              type="text"
              placeholder="Search documents..."
              bind:value={searchQuery}
              class="flex-1 rounded-md border border-slate-600 bg-slate-900 px-4 py-2 text-slate-100 placeholder-slate-500 focus:border-rose-500 focus:outline-none"
            />
            {#if searching}
              <span class="text-sm text-slate-500">Searching...</span>
            {/if}
          </div>

          <section class="flex-1 overflow-y-auto p-6">
            {#if results.length === 0}
              <p class="text-sm italic text-slate-500">
                {searchQuery ? "No results found" : "Start typing to search"}
              </p>
            {:else}
              <ul class="space-y-4">
                {#each results as result}
                  <li class="rounded-lg border border-slate-700 bg-slate-800 p-4">
                    <div class="mb-2 flex items-center justify-between">
                      <h3 class="font-medium text-rose-500">{result.document.name}</h3>
                      <span class="rounded bg-slate-700 px-2 py-0.5 text-xs text-slate-400">
                        {getCollectionName(result.collection_id)}
                      </span>
                    </div>
                    <p class="text-sm text-slate-400">{result.snippet}</p>
                    <span class="mt-2 inline-block text-xs text-slate-600">Score: {result.score.toFixed(2)}</span>
                  </li>
                {/each}
              </ul>
            {/if}
          </section>
        </div>
      </div>
    {:else if activeTab === "files"}
      <!-- Files Tab -->
      <div class="flex h-full">
        <Sidebar title="Collections">
          <div class="mb-4 flex gap-2">
            <input
              type="text"
              placeholder="New collection..."
              bind:value={newCollectionName}
              onkeydown={(e) => e.key === "Enter" && createCollection()}
              class="min-w-0 flex-1 rounded-md border border-slate-600 bg-slate-900 px-3 py-1.5 text-sm text-slate-100 placeholder-slate-500 focus:border-rose-500 focus:outline-none"
            />
            <button
              onclick={createCollection}
              class="rounded-md bg-rose-600 px-3 py-1.5 font-medium text-white hover:bg-rose-700"
            >
              +
            </button>
          </div>
          {#if collections.length === 0}
            <p class="text-sm italic text-slate-500">No collections yet</p>
          {:else}
            <ul class="space-y-1">
              {#each collections as collection}
                <li
                  class="group flex cursor-pointer items-center justify-between rounded px-3 py-2 text-sm {selectedCollection === collection.id
                    ? 'bg-rose-600/20 text-rose-400'
                    : 'hover:bg-slate-700'}"
                >
                  <button
                    type="button"
                    onclick={() => selectCollection(collection.id)}
                    class="flex-1 truncate text-left"
                  >
                    {collection.name}
                  </button>
                  <button
                    type="button"
                    onclick={(e) => deleteCollection(collection.id, e)}
                    class="ml-2 hidden text-slate-500 hover:text-red-400 group-hover:block"
                    title="Delete collection"
                  >
                    x
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        </Sidebar>

        <!-- Documents Area -->
        <section class="flex-1 overflow-y-auto p-6">
          <div class="mb-4 flex items-center justify-between">
            <h2 class="text-sm font-medium text-slate-400">
              {selectedCollection ? "Documents" : "Select a collection"}
            </h2>
            <button
              onclick={importPdf}
              disabled={importing || !selectedCollection}
              class="rounded-md bg-rose-600 px-4 py-2 text-sm font-medium text-white hover:bg-rose-700 disabled:opacity-60"
            >
              {importing ? "Importing..." : "Import PDF"}
            </button>
          </div>
          {#if documents.length === 0}
            <p class="text-sm italic text-slate-500">No documents yet</p>
          {:else}
            <ul class="space-y-2">
              {#each documents as doc}
                <li class="group flex items-center justify-between rounded-lg border border-slate-700 bg-slate-800 px-4 py-3">
                  <div>
                    <span class="text-slate-200">{doc.name}</span>
                    <span class="ml-2 text-xs text-slate-500">{doc.page_count} pages</span>
                  </div>
                  <button
                    onclick={() => deleteDocument(doc.id)}
                    class="hidden text-slate-500 hover:text-red-400 group-hover:block"
                    title="Delete document"
                  >
                    x
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        </section>
      </div>
    {/if}
  </div>
</main>
