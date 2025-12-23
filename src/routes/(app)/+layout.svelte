<script lang="ts">
	import { page } from '$app/stores';
	import { resolve } from '$app/paths';
	import { invoke } from '@tauri-apps/api/core';
	import { onMount } from 'svelte';
	import SetupWizard from '$lib/components/SetupWizard.svelte';
	import BootLoader from '$lib/components/BootLoader.svelte';
	import BootDownloader from '$lib/components/BootDownloader.svelte';

	let { children } = $props();

	type BootStatus = {
		embedding_configured: boolean;
		embedding_model_id: string | null;
		embedding_downloaded: boolean;
	};

	// App state machine
	type AppPhase =
		| { state: 'booting' }
		| { state: 'setup-required' }
		| { state: 'download-required'; modelId: string; modelName: string }
		| { state: 'embedder-failed'; modelId: string; error: string }
		| { state: 'ready' };

	let appPhase = $state<AppPhase>({ state: 'booting' });

	type Tab = { id: string; label: string; href: string };
	const tabs: Tab[] = [
		{ id: 'trajectory', label: 'Trajectory', href: '/trajectory' },
		{ id: 'search', label: 'Search', href: '/search' },
		{ id: 'files', label: 'Files', href: '/files' },
		{ id: 'settings', label: 'Settings', href: '/settings' },
	];

	const currentTab = $derived($page.url.pathname.split('/')[1] || 'search');

	function handleSetupComplete() {
		appPhase = { state: 'ready' };
	}

	onMount(async () => {
		console.log('Layout mounted, fetching boot status...');

		try {
			// Get boot status from backend
			const status = await invoke<BootStatus>('get_boot_status');
			console.log('Boot status:', status);

			if (!status.embedding_configured) {
				// No embedding model configured - show setup wizard
				appPhase = { state: 'setup-required' };
			} else if (!status.embedding_downloaded) {
				// Embedding configured but not downloaded - show download UI
				// We need the model name, fetch it
				const models = await invoke<{ id: string; name: string }[]>(
					'get_available_embedding_models',
				);
				const model = models.find((m) => m.id === status.embedding_model_id);
				appPhase = {
					state: 'download-required',
					modelId: status.embedding_model_id!,
					modelName: model?.name ?? 'Embedding Model',
				};
			} else {
				// Everything ready
				appPhase = { state: 'ready' };
			}
		} catch (e) {
			console.error('Failed to get boot status:', e);
			// Fallback to ready state
			appPhase = { state: 'ready' };
		}
	});
</script>

{#if appPhase.state === 'booting'}
	<BootLoader />
{:else if appPhase.state === 'setup-required'}
	<SetupWizard onComplete={handleSetupComplete} />
{:else if appPhase.state === 'download-required'}
	<BootDownloader
		modelId={appPhase.modelId}
		modelName={appPhase.modelName}
		onComplete={handleSetupComplete}
	/>
{:else if appPhase.state === 'embedder-failed'}
	<div class="flex h-screen items-center justify-center bg-slate-900">
		<div class="max-w-md text-center">
			<h1 class="mb-4 text-xl text-red-400">Failed to load embedding model</h1>
			<p class="mb-4 text-slate-400">{appPhase.error}</p>
			<button
				onclick={() => {
					appPhase = { state: 'setup-required' };
				}}
				class="rounded bg-rose-600 px-4 py-2 text-white hover:bg-rose-700"
			>
				Reconfigure Embedding Model
			</button>
		</div>
	</div>
{:else}
	<main class="flex h-screen flex-col bg-slate-900 text-slate-100">
		<!-- Tab Navigation -->
		<nav class="flex border-b border-slate-700 bg-slate-800">
			{#each tabs as tab (tab.id)}
				<a
					href={resolve(tab.href as '/')}
					class="px-6 py-3 text-sm font-medium transition-colors {currentTab ===
					tab.id
						? 'border-b-2 border-rose-500 text-rose-500'
						: 'text-slate-400 hover:text-slate-200'}"
				>
					{tab.label}
				</a>
			{/each}
		</nav>

		<!-- Tab Content -->
		<div class="flex-1 overflow-hidden">
			{@render children()}
		</div>
	</main>
{/if}
