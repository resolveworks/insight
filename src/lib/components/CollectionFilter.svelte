<script lang="ts">
	import type { Collection } from '$lib/stores/collections.svelte';

	type Props = {
		collections: Collection[];
		selected: Collection[];
		onAdd: (id: string) => void;
		onRemove: (id: string) => void;
	};

	let { collections, selected, onAdd, onRemove }: Props = $props();

	let search = $state('');

	const selectedIds = $derived(new Set(selected.map((c) => c.id)));

	const suggestions = $derived.by(() => {
		const q = search.trim().toLowerCase();
		return collections.filter((c) => {
			if (selectedIds.has(c.id)) return false;
			if (!q) return true;
			return c.name.toLowerCase().includes(q);
		});
	});

	function add(id: string) {
		onAdd(id);
		search = '';
	}

	function handleKey(e: KeyboardEvent) {
		if (e.key === 'Enter' && suggestions.length > 0) {
			e.preventDefault();
			add(suggestions[0].id);
		} else if (e.key === 'Backspace' && search === '' && selected.length > 0) {
			onRemove(selected[selected.length - 1].id);
		}
	}
</script>

<div class="border-b border-neutral-200 p-4">
	<div class="mb-2 flex items-baseline justify-between gap-2">
		<h3 class="text-xs font-medium uppercase tracking-wide text-neutral-500">
			Collections
		</h3>
		<span class="text-xs text-neutral-400">
			{selected.length} of {collections.length}
		</span>
	</div>

	{#if collections.length === 0}
		<p class="text-sm italic text-neutral-400">No collections</p>
	{:else}
		<div
			class="rounded-md border border-neutral-300 bg-surface-bright focus-within:border-secondary-400"
		>
			{#if selected.length > 0}
				<div class="flex flex-wrap gap-1 p-1.5 pb-0">
					{#each selected as c (c.id)}
						<span
							class="flex items-center gap-1 rounded bg-neutral-200 py-0.5 pl-2 pr-1 text-xs text-neutral-700"
						>
							<span class="max-w-[140px] truncate">{c.name}</span>
							<span class="text-neutral-400">{c.document_count}</span>
							<button
								type="button"
								onclick={() => onRemove(c.id)}
								aria-label="Remove {c.name}"
								class="flex h-4 w-4 items-center justify-center rounded text-neutral-500 hover:bg-neutral-300 hover:text-neutral-800"
							>
								×
							</button>
						</span>
					{/each}
				</div>
			{/if}
			<input
				type="text"
				bind:value={search}
				onkeydown={handleKey}
				placeholder={selected.length === 0
					? 'Search collections...'
					: 'Add more...'}
				class="w-full bg-transparent px-2 py-1.5 text-sm text-neutral-800 placeholder-neutral-400 focus:outline-none"
			/>
		</div>

		<ul class="mt-2 max-h-96 space-y-0.5 overflow-y-auto">
			{#each suggestions as c (c.id)}
				<li>
					<button
						type="button"
						onclick={() => add(c.id)}
						class="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm text-neutral-700 transition hover:bg-neutral-300"
					>
						<span class="truncate">{c.name}</span>
						<span class="ml-auto text-xs text-neutral-400">
							{c.document_count}
						</span>
					</button>
				</li>
			{:else}
				<li class="px-2 py-2 text-sm italic text-neutral-400">
					{search.trim() !== '' ? 'No matches' : 'Nothing left to add'}
				</li>
			{/each}
		</ul>
	{/if}
</div>
