<script lang="ts">
	import { page } from '$app/stores';
	import { resolve } from '$app/paths';
	import CenteredLayout from '$lib/components/CenteredLayout.svelte';
	import DownloadProgress from '$lib/components/DownloadProgress.svelte';
	import LoadingSpinner from '$lib/components/LoadingSpinner.svelte';
	import Button from '$lib/components/Button.svelte';
	import { getEmbeddingState } from '$lib/stores/provider-state.svelte';

	let { children } = $props();

	let embeddingState = $derived(getEmbeddingState());

	type Tab = { id: string; label: string; href: string };
	const tabs: Tab[] = [
		{ id: 'research', label: 'Research', href: '/research' },
		{ id: 'files', label: 'Files', href: '/files' },
		{ id: 'settings', label: 'Settings', href: '/settings' },
	];

	const currentTab = $derived($page.url.pathname.split('/')[1] || 'research');
</script>

{#if embeddingState.ready}
	<main class="flex h-screen flex-col bg-surface-dim text-neutral-800">
		<nav class="flex border-b border-neutral-300 bg-neutral-700">
			{#each tabs as tab (tab.id)}
				<a
					href={resolve(tab.href as '/')}
					class="px-6 py-3 text-sm font-medium transition-colors {currentTab ===
					tab.id
						? 'border-b-2 border-primary-500 text-surface'
						: 'text-neutral-300 hover:text-surface'}"
				>
					{tab.label}
				</a>
			{/each}
		</nav>
		<div class="flex-1 overflow-hidden">
			{@render children()}
		</div>
	</main>
{:else if embeddingState.error}
	<CenteredLayout width="sm">
		<div class="text-center">
			<h1 class="mb-2 text-xl text-error">Failed to load embedding model</h1>
			<p class="mb-6 text-sm text-neutral-500">{embeddingState.error}</p>
			<Button color="accent" size="lg" onclick={() => window.location.reload()}>
				Retry
			</Button>
		</div>
	</CenteredLayout>
{:else if embeddingState.progress}
	<CenteredLayout width="sm">
		<DownloadProgress providerType="embedding" />
		<p class="mt-6 text-center text-sm text-neutral-500">
			This is a one-time download. The model will be cached locally.
		</p>
	</CenteredLayout>
{:else}
	<CenteredLayout width="sm">
		<div class="text-center">
			<h1 class="mb-2 text-xl text-neutral-700">Loading model</h1>
			<p class="mb-6 text-sm text-neutral-500">
				Initializing the embedding model for search
			</p>
			<LoadingSpinner size="md" color="accent" class="mx-auto" />
		</div>
	</CenteredLayout>
{/if}
