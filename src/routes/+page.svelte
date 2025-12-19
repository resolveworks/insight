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

<main class="flex h-screen flex-col bg-slate-900 text-slate-100">
  <div class="flex gap-2 bg-slate-800 p-4">
    <input
      type="text"
      placeholder="Search documents..."
      bind:value={searchQuery}
      onkeydown={(e) => e.key === "Enter" && search()}
      class="flex-1 rounded-md border border-slate-600 bg-slate-900 px-4 py-2 text-slate-100 placeholder-slate-500 focus:border-rose-500 focus:outline-none"
    />
    <button
      onclick={search}
      class="rounded-md bg-rose-600 px-6 py-2 font-medium text-white hover:bg-rose-700 disabled:cursor-not-allowed disabled:opacity-60"
    >
      Search
    </button>
    <button
      onclick={importPdf}
      disabled={importing}
      class="rounded-md bg-rose-600 px-6 py-2 font-medium text-white hover:bg-rose-700 disabled:cursor-not-allowed disabled:opacity-60"
    >
      {importing ? "Importing..." : "Import PDF"}
    </button>
  </div>

  <div class="flex flex-1 overflow-hidden">
    <aside class="flex w-72 flex-col gap-4 overflow-y-auto border-r border-slate-700 bg-slate-800 p-4">
      <section>
        <h2 class="mb-2 text-xs font-medium uppercase tracking-wide text-slate-400">Collections</h2>
        <div class="mb-2 flex gap-2">
          <input
            type="text"
            placeholder="New collection..."
            bind:value={newCollectionName}
            onkeydown={(e) => e.key === "Enter" && createCollection()}
            class="flex-1 rounded-md border border-slate-600 bg-slate-900 px-3 py-1.5 text-sm text-slate-100 placeholder-slate-500 focus:border-rose-500 focus:outline-none"
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
              <li class="cursor-pointer truncate rounded px-2 py-1.5 text-sm hover:bg-slate-700">
                {collection.name}
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      <section>
        <h2 class="mb-2 text-xs font-medium uppercase tracking-wide text-slate-400">Imported Documents</h2>
        {#if documents.length === 0}
          <p class="text-sm italic text-slate-500">No documents imported</p>
        {:else}
          <ul class="space-y-1">
            {#each documents as doc}
              <li
                title={`${doc.page_count} pages`}
                class="cursor-pointer truncate rounded px-2 py-1.5 text-sm hover:bg-slate-700"
              >
                {doc.name}
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    </aside>

    <section class="flex-1 overflow-y-auto p-6">
      {#if results.length === 0}
        <p class="text-sm italic text-slate-500">
          {searchQuery ? "No results found" : "Enter a search query to find documents"}
        </p>
      {:else}
        <ul class="space-y-4">
          {#each results as result}
            <li class="rounded-lg border border-slate-700 bg-slate-800 p-4">
              <h3 class="mb-2 font-medium text-rose-500">{result.document.name}</h3>
              <p class="text-sm leading-relaxed text-slate-400">{result.snippet}</p>
              <span class="mt-2 inline-block text-xs text-slate-600">Score: {result.score.toFixed(2)}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  </div>
</main>
