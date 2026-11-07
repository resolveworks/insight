<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import type { LayoutProps } from './$types';

	let { data, children }: LayoutProps = $props();

	const collectionId = $derived(page.params.collectionId ?? '');
	const collection = $derived(data.collection);

	const tabs = [
		{ id: 'research', label: 'Research' },
		{ id: 'documents', label: 'Documents' },
	];
	const currentTab = $derived(page.url.pathname.split('/')[3]);
</script>

<div class="flex h-full flex-col bg-surface">
	<header class="border-b border-neutral-200 bg-surface-bright px-6 py-4">
		<div class="min-w-0">
			<p class="text-xs uppercase tracking-wide text-neutral-400">Collection</p>
			<h1 class="truncate text-xl font-medium text-neutral-800">
				{collection.name}
			</h1>
		</div>
		<nav aria-label="Collection sections" class="mt-3 flex gap-1">
			{#each tabs as tab (tab.id)}
				<a
					href={resolve(`collections/${collectionId}/${tab.id}`)}
					aria-current={currentTab === tab.id ? 'page' : undefined}
					class="rounded-t-md border-b-2 px-4 py-2 text-sm font-medium transition-colors {currentTab ===
					tab.id
						? 'border-primary-500 text-primary-600'
						: 'border-transparent text-neutral-500 hover:text-neutral-800'}"
				>
					{tab.label}
				</a>
			{/each}
		</nav>
	</header>
	<div class="flex-1 overflow-hidden">
		{@render children()}
	</div>
</div>
