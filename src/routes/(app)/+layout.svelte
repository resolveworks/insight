<script lang="ts">
	import { page } from '$app/stores';
	import { resolve } from '$app/paths';
	import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	import { onDestroy, onMount } from 'svelte';
	import SetupWizard from '$lib/components/SetupWizard.svelte';
	import BootLoader from '$lib/components/BootLoader.svelte';

	let { children } = $props();

	// Boot phase types matching backend BootPhase enum (src-tauri/src/core/mod.rs)
	type BootPhaseEvent =
		| {
				phase: 'StorageReady';
				embedding_configured: boolean;
				embedding_model_id: string | null;
		  }
		| { phase: 'EmbedderLoading'; model_id: string; model_name: string }
		| { phase: 'EmbedderReady'; model_id: string }
		| { phase: 'EmbedderFailed'; model_id: string; error: string }
		| { phase: 'AppReady' };

	// App state machine
	type AppPhase =
		| { state: 'booting' }
		| { state: 'setup-required' }
		| { state: 'loading-embedder'; modelName: string }
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

	let unlistenBootPhase: UnlistenFn;

	function handleSetupComplete() {
		appPhase = { state: 'ready' };
	}

	onMount(async () => {
		unlistenBootPhase = await listen<BootPhaseEvent>('boot-phase', (event) => {
			const phase = event.payload;

			switch (phase.phase) {
				case 'StorageReady':
					if (!phase.embedding_configured) {
						appPhase = { state: 'setup-required' };
					}
					break;

				case 'EmbedderLoading':
					appPhase = {
						state: 'loading-embedder',
						modelName: phase.model_name,
					};
					break;

				case 'EmbedderReady':
					break;

				case 'AppReady':
					if (appPhase.state !== 'setup-required') {
						appPhase = { state: 'ready' };
					}
					break;

				case 'EmbedderFailed':
					appPhase = {
						state: 'embedder-failed',
						modelId: phase.model_id,
						error: phase.error,
					};
					break;
			}
		});
	});

	onDestroy(() => {
		unlistenBootPhase?.();
	});
</script>

{#if appPhase.state === 'booting'}
	<BootLoader phase="storage" />
{:else if appPhase.state === 'setup-required'}
	<SetupWizard onComplete={handleSetupComplete} />
{:else if appPhase.state === 'loading-embedder'}
	<BootLoader phase="embedder" modelName={appPhase.modelName} />
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
