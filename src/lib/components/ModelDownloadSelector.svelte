<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { listen } from '@tauri-apps/api/event';
	import { onMount } from 'svelte';
	import type { ModelSelectorConfig } from '$lib/models/config';
	import {
		getDownloadState,
		startDownload,
		clearDownload,
		setLoadedModel,
	} from '$lib/stores/model-state.svelte';
	import Button from './Button.svelte';
	import ErrorAlert from './ErrorAlert.svelte';

	export interface ModelInfo {
		id: string;
		name: string;
		description: string;
		size_gb: number;
		dimensions?: number;
	}

	type Status = 'loading' | 'idle' | 'configuring';

	type Props = {
		config: ModelSelectorConfig;
		onConfigured?: (modelId: string | null) => void;
	};

	let { config, onConfigured }: Props = $props();

	let models = $state<ModelInfo[]>([]);
	let selectedId = $state<string | null>(null);
	let activeId = $state<string | null>(null);
	let isDownloaded = $state(false);
	let status = $state<Status>('loading');
	let error = $state<string | null>(null);

	// Get download state from global store (persists across navigation)
	let downloadState = $derived(getDownloadState(config));
	let isDownloading = $derived(downloadState.status === 'downloading');

	// Derived state
	let canDownload = $derived(
		selectedId && !isDownloaded && status === 'idle' && !isDownloading,
	);
	let canConfigure = $derived(
		isDownloaded &&
			selectedId !== activeId &&
			status === 'idle' &&
			!isDownloading,
	);
	let isActive = $derived(selectedId === activeId);

	// Button color based on config accent
	let buttonColor = $derived(
		config.accentColor === 'emerald' ? 'emerald' : 'amber',
	) as 'amber' | 'emerald';

	// Color classes based on accent
	let accentClasses = $derived({
		border:
			config.accentColor === 'amber'
				? 'border-amber-500'
				: config.accentColor === 'emerald'
					? 'border-emerald-500'
					: 'border-neutral-500',
		bg:
			config.accentColor === 'amber'
				? 'bg-amber-900/30'
				: config.accentColor === 'emerald'
					? 'bg-emerald-900/30'
					: 'bg-neutral-800',
		text:
			config.accentColor === 'amber'
				? 'text-amber-500'
				: config.accentColor === 'emerald'
					? 'text-emerald-500'
					: 'text-neutral-500',
		btn:
			config.accentColor === 'amber'
				? 'bg-amber-500 hover:bg-amber-600'
				: config.accentColor === 'emerald'
					? 'bg-emerald-500 hover:bg-emerald-600'
					: 'bg-neutral-500 hover:bg-neutral-600',
		progress:
			config.accentColor === 'amber'
				? 'bg-amber-500'
				: config.accentColor === 'emerald'
					? 'bg-emerald-500'
					: 'bg-neutral-500',
	});

	async function load() {
		status = 'loading';
		error = null;

		try {
			models = await invoke<ModelInfo[]>(config.listCommand);
			if (models.length > 0) {
				activeId = await invoke<string | null>(config.currentCommand);
				selectedId = activeId ?? models[0].id;
				await checkStatus();
			}
		} catch (e) {
			error = `Failed to load models: ${e}`;
		} finally {
			if (status === 'loading') status = 'idle';
		}
	}

	async function checkStatus() {
		if (!selectedId) return;

		try {
			const result = await invoke<{ status: string }>(config.statusCommand, {
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
		startDownload(config, selectedId);

		try {
			// Listen for completion to update local downloaded state
			const unlisten = await listen(config.completeEvent, () => {
				isDownloaded = true;
				unlisten();
			});

			await invoke(config.downloadCommand, { modelId: selectedId });
		} catch (e) {
			error = `Download failed: ${e}`;
			clearDownload(config);
		}
	}

	async function configure() {
		if (!selectedId) return;

		status = 'configuring';
		error = null;

		try {
			await invoke(config.configureCommand, { modelId: selectedId });
			activeId = selectedId;
			setLoadedModel(config, selectedId);
			onConfigured?.(selectedId);
		} catch (e) {
			error = `Failed to configure: ${e}`;
		} finally {
			status = 'idle';
		}
	}

	async function disable() {
		status = 'configuring';
		error = null;

		try {
			await invoke(config.configureCommand, { modelId: null });
			activeId = null;
			onConfigured?.(null);
		} catch (e) {
			error = `Failed to disable: ${e}`;
		} finally {
			status = 'idle';
		}
	}

	function formatBytes(bytes: number): string {
		if (bytes === 0) return '0 B';
		const k = 1024;
		const sizes = ['B', 'KB', 'MB', 'GB'];
		const i = Math.floor(Math.log(bytes) / Math.log(k));
		return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
	}

	onMount(load);
</script>

<div>
	{#if status === 'loading'}
		<p class="text-neutral-400 text-center py-4">Loading models...</p>
	{:else if isDownloading}
		<div class="text-center">
			<h3 class="text-lg text-neutral-300 mb-4">Downloading {config.title}</h3>
			{#if downloadState.progress}
				<p class="text-sm text-neutral-400 mb-2">
					File {downloadState.progress.file_index} of {downloadState.progress
						.total_files}: {downloadState.progress.file.split('/').pop()}
				</p>
				<div class="h-2 bg-neutral-700 rounded-full overflow-hidden mb-2">
					<div
						class="h-full transition-[width] duration-300 {accentClasses.progress}"
						style="width: {downloadState.progress.overall_progress * 100}%"
					></div>
				</div>
				<p class="text-xs text-neutral-500">
					{formatBytes(downloadState.progress.downloaded)} / {formatBytes(
						downloadState.progress.total,
					)}
					({Math.round(downloadState.progress.overall_progress * 100)}%)
				</p>
			{:else}
				<p class="text-sm text-neutral-400 mb-2">Starting download...</p>
			{/if}
		</div>
	{:else}
		<div
			class="flex items-center gap-2 px-4 py-3 rounded-lg border mb-4 text-sm {activeId
				? `${accentClasses.border} ${accentClasses.bg} text-neutral-200`
				: 'border-neutral-600 bg-neutral-800 text-neutral-400'}"
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
					class="ml-auto text-xs text-neutral-400 hover:text-neutral-200 cursor-pointer"
					onclick={disable}
					disabled={status === 'configuring'}
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
						: 'border-neutral-600 hover:border-neutral-500 bg-transparent'}"
					onclick={() => select(model.id)}
				>
					<div class="flex flex-col gap-0.5">
						<span class="font-medium text-neutral-200">{model.name}</span>
						<span class="text-sm text-neutral-400">{model.description}</span>
						{#if model.dimensions}
							<span class="text-xs text-neutral-500 mt-1"
								>{model.dimensions} dimensions</span
							>
						{/if}
					</div>
					<div class="flex flex-col items-end gap-1 ml-4">
						<span class="text-sm text-neutral-500">{model.size_gb} GB</span>
						{#if model.id === activeId}
							<span class="text-xs {accentClasses.text}">Active</span>
						{:else if model.id === selectedId && isDownloaded}
							<span class="text-xs text-neutral-400">Downloaded</span>
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
					disabled={status === 'configuring'}
					loading={status === 'configuring'}
				>
					{status === 'configuring' ? 'Loading...' : 'Activate Model'}
				</Button>
				{#if status === 'configuring'}
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
