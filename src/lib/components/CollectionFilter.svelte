<script lang="ts">
	import type { SvelteSet } from 'svelte/reactivity';
	import type { Collection } from '$lib/stores/collections.svelte';

	type Props = {
		collections: Collection[];
		selected: SvelteSet<string>;
	};

	let { collections, selected }: Props = $props();

	let search = $state('');

	const filtered = $derived.by(() => {
		const q = search.trim().toLowerCase();
		if (!q) return collections;
		return collections.filter((c) => c.name.toLowerCase().includes(q));
	});

	function toggle(id: string) {
		if (selected.has(id)) {
			selected.delete(id);
		} else {
			selected.add(id);
		}
	}

	function clear() {
		selected.clear();
	}
</script>

<div class="border-b border-primary-700 p-4">
	<div class="mb-2 flex items-baseline justify-between gap-2">
		<h3 class="text-xs font-medium uppercase tracking-wide text-primary-200">
			Collections
		</h3>
		<span class="text-xs text-primary-300">
			{selected.size === 0
				? `All ${collections.length}`
				: `${selected.size} of ${collections.length}`}
		</span>
	</div>

	{#if collections.length === 0}
		<p class="text-sm italic text-primary-300">No collections</p>
	{:else}
		{#if collections.length > 5}
			<input
				type="text"
				bind:value={search}
				placeholder="Search..."
				class="mb-2 w-full rounded-md border border-primary-700 bg-primary-700 px-2 py-1.5 text-sm text-surface placeholder-primary-300 focus:border-secondary-400 focus:outline-none"
			/>
		{/if}

		<ul class="space-y-1">
			{#each filtered as collection (collection.id)}
				<li>
					<label
						class="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-primary-500"
					>
						<input
							type="checkbox"
							checked={selected.has(collection.id)}
							onchange={() => toggle(collection.id)}
							class="h-4 w-4 rounded border-primary-400 bg-primary-700 text-secondary-400 focus:ring-secondary-400"
						/>
						<span
							class="truncate {selected.has(collection.id)
								? 'text-surface'
								: 'text-primary-100'}"
						>
							{collection.name}
						</span>
						<span class="ml-auto text-xs text-primary-300">
							{collection.document_count}
						</span>
					</label>
				</li>
			{/each}
			{#if filtered.length === 0}
				<li class="px-2 py-2 text-sm italic text-primary-300">No matches</li>
			{/if}
		</ul>

		{#if selected.size > 0}
			<button
				onclick={clear}
				class="mt-2 text-xs text-primary-200 hover:text-surface"
			>
				Clear selection
			</button>
		{/if}
	{/if}
</div>
