<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import LoadingSpinner from '#lib/components/LoadingSpinner.svelte';
	import * as collections from '#lib/stores/collections.svelte.js';

	let { children } = $props();

	const collectionId = $derived(page.params.collectionId ?? '');
	const collection = $derived(collections.getCollection(collectionId));
	const loading = $derived(collections.isLoading());

	const tabs = [
		{ id: 'research', label: 'Research' },
		{ id: 'documents', label: 'Documents' },
	];
	const currentTab = $derived(page.url.pathname.split('/')[3]);
</script>

{#if collection}
	<div class="flex h-full flex-col bg-surface">
		<header class="border-b border-neutral-200 bg-surface-bright px-6 py-4">
			<div class="min-w-0">
				<p class="text-xs uppercase tracking-wide text-neutral-400">
					Collection
				</p>
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
{:else if loading}
	<div class="flex h-full items-center justify-center bg-surface">
		<LoadingSpinner size="lg" />
	</div>
{:else}
	<div
		class="flex h-full flex-col items-center justify-center gap-2 bg-surface"
	>
		<p class="text-neutral-500">Collection not found</p>
		<a
			href={resolve('collections')}
			class="text-sm text-neutral-500 transition-colors hover:text-primary-600"
		>
			Back to collections
		</a>
	</div>
{/if}
