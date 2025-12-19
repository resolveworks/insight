<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";

  type Tab = "trajectory" | "search" | "files";
  let activeTab = $state<Tab>("search");

  let searchQuery = $state("");
  let results = $state<any[]>([]);
  let collections = $state<any[]>([]);
  let documents = $state<any[]>([]);
  let importing = $state(false);
  let newCollectionName = $state("");
  let selectedCollection = $state<string | null>(null);

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
        const doc = await invoke("import_pdf", { path, collectionId: selectedCollection });
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
      <div class="flex flex-col h-full">
        <div class="flex gap-2 p-4">
          <input
            type="text"
            placeholder="Search documents..."
            bind:value={searchQuery}
            onkeydown={(e) => e.key === "Enter" && search()}
            class="flex-1 rounded-md border border-slate-600 bg-slate-900 px-4 py-2 text-slate-100 placeholder-slate-500 focus:border-rose-500 focus:outline-none"
          />
          <button
            onclick={search}
            class="rounded-md bg-rose-600 px-6 py-2 font-medium text-white hover:bg-rose-700"
          >
            Search
          </button>
        </div>

        <section class="flex-1 overflow-y-auto p-6">
          {#if results.length === 0}
            <p class="text-sm italic text-slate-500">
              {searchQuery ? "No results found" : "Enter a search query"}
            </p>
          {:else}
            <ul class="space-y-4">
              {#each results as result}
                <li class="rounded-lg border border-slate-700 bg-slate-800 p-4">
                  <h3 class="mb-2 font-medium text-rose-500">{result.document.name}</h3>
                  <p class="text-sm text-slate-400">{result.snippet}</p>
                  <span class="mt-2 inline-block text-xs text-slate-600">Score: {result.score.toFixed(2)}</span>
                </li>
              {/each}
            </ul>
          {/if}
        </section>
      </div>
    {:else if activeTab === "files"}
      <!-- Files Tab -->
      <div class="flex h-full">
        <!-- Collections Sidebar -->
        <aside class="w-64 overflow-y-auto border-r border-slate-700 bg-slate-800 p-4">
          <h2 class="mb-3 text-xs font-medium uppercase tracking-wide text-slate-400">Collections</h2>
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
                  onclick={() => (selectedCollection = selectedCollection === collection.id ? null : collection.id)}
                  class="cursor-pointer truncate rounded px-3 py-2 text-sm {selectedCollection === collection.id
                    ? 'bg-rose-600/20 text-rose-400'
                    : 'hover:bg-slate-700'}"
                >
                  {collection.name}
                </li>
              {/each}
            </ul>
          {/if}
        </aside>

        <!-- Documents Area -->
        <section class="flex-1 overflow-y-auto p-6">
          <div class="mb-4 flex items-center justify-between">
            <h2 class="text-sm font-medium text-slate-400">Documents</h2>
            <button
              onclick={importPdf}
              disabled={importing}
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
                <li class="rounded-lg border border-slate-700 bg-slate-800 px-4 py-3">
                  <span class="text-slate-200">{doc.name}</span>
                  <span class="ml-2 text-xs text-slate-500">{doc.page_count} pages</span>
                </li>
              {/each}
            </ul>
          {/if}
        </section>
      </div>
    {/if}
  </div>
</main>
