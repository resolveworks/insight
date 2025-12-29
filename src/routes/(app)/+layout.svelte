<script lang="ts">
	import { page } from '$app/stores';
	import { resolve } from '$app/paths';
	import CenteredLayout from '$lib/components/CenteredLayout.svelte';
	import DownloadProgress from '$lib/components/DownloadProgress.svelte';
	import LoadingSpinner from '$lib/components/LoadingSpinner.svelte';
	import Button from '$lib/components/Button.svelte';
	import { getEmbeddingState } from '$lib/stores/model-state.svelte';

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
			<svg
				class="mx-auto mb-6 h-12 w-12 text-error"
				viewBox="0 0 24 24"
				fill="none"
				stroke="currentColor"
				stroke-width="2"
			>
				<path
					stroke-linecap="round"
					stroke-linejoin="round"
					d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
				/>
			</svg>
			<h1 class="mb-2 text-xl text-error">Failed to load embedding model</h1>
			<p class="mb-6 text-sm text-neutral-500">{embeddingState.error}</p>
			<Button color="accent" size="lg" onclick={() => window.location.reload()}>
				Retry
			</Button>
		</div>
	</CenteredLayout>
{:else if embeddingState.progress}
	<CenteredLayout width="sm">
		<div class="mb-6 text-center">
			<svg
				class="mx-auto h-12 w-12 text-tertiary-500"
				viewBox="0 0 24 24"
				fill="none"
				stroke="currentColor"
				stroke-width="2"
			>
				<path
					stroke-linecap="round"
					stroke-linejoin="round"
					d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4"
				/>
			</svg>
		</div>
		<DownloadProgress modelType="embedding" />
		<p class="mt-6 text-center text-sm text-neutral-500">
			This is a one-time download. The model will be cached locally.
		</p>
	</CenteredLayout>
{:else}
	<CenteredLayout width="sm">
		<div class="text-center">
			<LoadingSpinner size="lg" color="accent" />
			<p class="mt-6 text-sm text-neutral-500">Loading model...</p>
		</div>
	</CenteredLayout>
{/if}
