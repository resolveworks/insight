<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { onMount } from 'svelte';
	import type { ModelSelectorConfig } from '$lib/models/config';
	import {
		getProviderState,
		refreshProviderState,
	} from '$lib/stores/provider-state.svelte';
	import Button from './Button.svelte';
	import ErrorAlert from './ErrorAlert.svelte';
	import DownloadProgress from './DownloadProgress.svelte';

	export interface ModelInfo {
		id: string;
		name: string;
		description: string;
		size_gb: number;
		dimensions?: number;
	}

	type Props = {
		config: ModelSelectorConfig;
		onConfigured?: (modelId: string | null) => void;
	};

	let { config, onConfigured }: Props = $props();

	let models = $state<ModelInfo[]>([]);
	let selectedId = $state<string | null>(null);
	let activeId = $state<string | null>(null);
	let isDownloaded = $state(false);
	let listLoading = $state(true);
	let error = $state<string | null>(null);

	const providerState = $derived(getProviderState(config.providerType));
	const isDownloading = $derived(providerState.status.kind === 'downloading');
	const isConfiguring = $derived(providerState.status.kind === 'loading');
	const busy = $derived(isDownloading || isConfiguring);

	const canDownload = $derived(!!selectedId && !isDownloaded && !busy);
	const canConfigure = $derived(
		isDownloaded && selectedId !== activeId && !busy,
	);
	const isActive = $derived(selectedId === activeId);

	const buttonColor = $derived(
		config.accentColor === 'emerald' ? 'accent' : 'primary',
	) as 'primary' | 'accent';

	const accentClasses = $derived({
		border:
			config.accentColor === 'slate'
				? 'border-primary-500'
				: config.accentColor === 'emerald'
					? 'border-tertiary-500'
					: 'border-neutral-400',
		bg:
			config.accentColor === 'slate'
				? 'bg-primary-50'
				: config.accentColor === 'emerald'
					? 'bg-tertiary-50'
					: 'bg-neutral-100',
		text:
			config.accentColor === 'slate'
				? 'text-primary-600'
				: config.accentColor === 'emerald'
					? 'text-tertiary-600'
					: 'text-neutral-600',
	});

	async function load() {
		listLoading = true;
		error = null;
		try {
			models = await invoke<ModelInfo[]>('get_available_models', {
				modelType: config.providerType,
			});
			if (models.length > 0) {
				activeId = await invoke<string | null>('get_current_model', {
					modelType: config.providerType,
				});
				selectedId = activeId ?? models[0].id;
				await checkStatus();
			}
		} catch (e) {
			error = `Failed to load models: ${e}`;
		} finally {
			listLoading = false;
		}
	}

	async function checkStatus() {
		if (!selectedId) return;
		try {
			const result = await invoke<{ status: string }>('get_model_status', {
				modelType: config.providerType,
				modelId: selectedId,
			});
			isDownloaded = result.status === 'Ready';
		} catch (e) {
			error = `Failed to check status: ${e}`;
		}
	}

	async function select(id: string) {
		if (id === selectedId) return;
		selectedId = id;
		await checkStatus();
	}

	async function download() {
		if (!selectedId) return;
		error = null;
		try {
			await invoke('download_model', {
				modelType: config.providerType,
				modelId: selectedId,
			});
			isDownloaded = true;
		} catch (e) {
			error = `Download failed: ${e}`;
		} finally {
			// Download doesn't emit a terminal status event — re-sync so the
			// UI leaves the progress view.
			await refreshProviderState(config.providerType);
		}
	}

	async function configure() {
		if (!selectedId) return;
		error = null;
		try {
			await invoke('configure_model', {
				modelType: config.providerType,
				modelId: selectedId,
			});
			activeId = selectedId;
			onConfigured?.(selectedId);
		} catch (e) {
			error = `Failed to configure: ${e}`;
		}
	}

	async function disable() {
		error = null;
		try {
			await invoke('configure_model', {
				modelType: config.providerType,
				modelId: null,
			});
			activeId = null;
			onConfigured?.(null);
			await refreshProviderState(config.providerType);
		} catch (e) {
			error = `Failed to disable: ${e}`;
		}
	}

	onMount(load);
</script>

<div>
	{#if listLoading}
		<p class="text-neutral-500 text-center py-4">Loading models...</p>
	{:else if isDownloading}
		<DownloadProgress
			providerType={config.providerType}
			title="Downloading {config.title}"
			accentColor={config.accentColor === 'emerald' ? 'accent' : 'primary'}
		/>
	{:else}
		<div
			class="flex items-center gap-2 px-4 py-3 rounded-lg border mb-4 text-sm {activeId
				? `${accentClasses.border} ${accentClasses.bg} text-neutral-700`
				: 'border-neutral-300 bg-surface-dim text-neutral-500'}"
		>
			{#if activeId}
				<svg
					class="w-5 h-5 shrink-0 {accentClasses.text}"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
				>
					<path
						stroke-linecap="round"
						stroke-linejoin="round"
						stroke-width="2"
						d="M5 13l4 4L19 7"
					/>
				</svg>
				<span>{models.find((m) => m.id === activeId)?.name} active</span>
				<button
					class="ml-auto text-xs text-neutral-500 hover:text-neutral-700 cursor-pointer"
					onclick={disable}
					disabled={isConfiguring}
				>
					Disable
				</button>
			{:else}
				<svg
					class="w-5 h-5 shrink-0"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
				>
					<path
						stroke-linecap="round"
						stroke-linejoin="round"
						stroke-width="2"
						d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
					/>
				</svg>
				<span>No model configured</span>
			{/if}
		</div>

		<div class="flex flex-col gap-2">
			{#each models as model (model.id)}
				<button
					class="flex justify-between items-center w-full p-3 rounded-lg border text-left cursor-pointer transition-colors duration-150 {selectedId ===
					model.id
						? `${accentClasses.border} ${accentClasses.bg}`
						: 'border-neutral-300 hover:border-neutral-400 bg-transparent'}"
					onclick={() => select(model.id)}
				>
					<div class="flex flex-col gap-0.5">
						<span class="font-medium text-neutral-800">{model.name}</span>
						<span class="text-sm text-neutral-500">{model.description}</span>
						{#if model.dimensions}
							<span class="text-xs text-neutral-400 mt-1"
								>{model.dimensions} dimensions</span
							>
						{/if}
					</div>
					<div class="flex flex-col items-end gap-1 ml-4">
						<span class="text-sm text-neutral-500">{model.size_gb} GB</span>
						{#if model.id === activeId}
							<span class="text-xs {accentClasses.text}">Active</span>
						{:else if model.id === selectedId && isDownloaded}
							<span class="text-xs text-neutral-500">Downloaded</span>
						{/if}
					</div>
				</button>
			{/each}
		</div>

		<div class="mt-6 flex flex-col items-center gap-3">
			{#if canDownload}
				<Button fullWidth color={buttonColor} onclick={download}>
					Download Model
				</Button>
			{:else if canConfigure}
				<Button
					fullWidth
					color={buttonColor}
					onclick={configure}
					disabled={isConfiguring}
					loading={isConfiguring}
				>
					{isConfiguring ? 'Loading...' : 'Activate Model'}
				</Button>
				{#if isConfiguring}
					<p class="text-xs text-neutral-500 text-center">
						This may take 20-30 seconds on first load
					</p>
				{/if}
			{:else if isActive}
				<p class="text-sm {accentClasses.text}">Model active</p>
			{:else if isDownloaded}
				<p class="text-sm {accentClasses.text}">Model ready</p>
			{/if}
		</div>

		{#if error}
			<div class="mt-4">
				<ErrorAlert>{error}</ErrorAlert>
			</div>
		{/if}
	{/if}
</div>
