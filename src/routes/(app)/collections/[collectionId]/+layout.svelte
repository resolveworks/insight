<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import LoadingSpinner from '#lib/components/LoadingSpinner.svelte';
	import * as collections from '#lib/stores/collections.svelte.js';

	let { children } = $props();

	const collectionId = $derived(page.params.collectionId ?? '');
	const collection = $derived(collections.getCollection(collectionId));
	const loading = $derived(collections.isLoading());

	const sectionTabs = [
		{ id: 'research', label: 'Research' },
		{ id: 'documents', label: 'Documents' },
	];
	const currentTab = $derived(page.route.id?.split('/').at(-1));
</script>

{#if collection}
	<div class="flex h-full flex-col bg-surface">
		<PageHeader eyebrow="Collection" title={collection.name}>
			{#snippet tabs()}
				<nav aria-label="Collection sections" class="flex gap-1">
					{#each sectionTabs as tab (tab.id)}
						<a
							href={resolve(`collections/${collectionId}/${tab.id}`)}
							aria-current={currentTab === tab.id ? 'page' : undefined}
							class="rounded-t-md border-b-2 px-4 py-2 text-sm font-medium transition-colors {currentTab ===
							tab.id
								? 'border-primary-500 text-primary-600'
								: 'border-transparent text-muted hover:text-neutral-800'}"
						>
							{tab.label}
						</a>
					{/each}
				</nav>
			{/snippet}
		</PageHeader>
		<div class="flex-1 overflow-hidden">
			{@render children()}
		</div>
	</div>
{:else if loading}
	<div class="flex h-full items-center justify-center bg-surface">
		<LoadingSpinner size="lg" ariaLabel="Loading collection" />
	</div>
{:else}
	<div
		class="flex h-full flex-col items-center justify-center gap-2 bg-surface"
	>
		<p class="text-muted">Collection not found</p>
		<a
			href={resolve('collections')}
			class="text-sm text-muted transition-colors hover:text-primary-600"
		>
			Back to collections
		</a>
	</div>
{/if}
