<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	import { onDestroy, onMount } from 'svelte';

	export interface EmbeddingModelInfo {
		id: string;
		name: string;
		description: string;
		size_gb: number;
		hf_repo_id: string;
		dimensions: number;
	}

	export interface EmbeddingModelStatus {
		status: 'NotDownloaded' | 'Ready';
	}

	export interface DownloadProgress {
		file: string;
		downloaded: number;
		total: number;
		overall_progress: number;
		file_index: number;
		total_files: number;
	}

	type Props = {
		onModelConfigured?: (modelId: string | null) => void;
		showTitle?: boolean;
	};

	let { onModelConfigured, showTitle = true }: Props = $props();

	let availableModels = $state<EmbeddingModelInfo[]>([]);
	let selectedModelId = $state<string | null>(null);
	let currentModelId = $state<string | null>(null);
	let modelStatus = $state<EmbeddingModelStatus['status']>('NotDownloaded');
	let downloadProgress = $state<DownloadProgress | null>(null);
	let isDownloading = $state(false);
	let isConfiguring = $state(false);
	let isLoading = $state(true);
	let error = $state<string | null>(null);

	let unlistenDownloadProgress: UnlistenFn | undefined;
	let unlistenDownloadComplete: UnlistenFn | undefined;

	async function loadAvailableModels() {
		try {
			availableModels = await invoke<EmbeddingModelInfo[]>(
				'get_available_embedding_models',
			);
			if (availableModels.length > 0 && !selectedModelId) {
				selectedModelId = availableModels[0].id;
			}
		} catch (e) {
			console.error('Failed to load embedding models:', e);
			error = `Failed to load models: ${e}`;
		}
	}

	async function loadCurrentModel() {
		try {
			currentModelId = await invoke<string | null>('get_current_embedding_model');
			if (currentModelId) {
				selectedModelId = currentModelId;
			}
		} catch (e) {
			console.error('Failed to get current embedding model:', e);
		}
	}

	async function checkModelStatus() {
		if (!selectedModelId) return;
		try {
			const status = await invoke<EmbeddingModelStatus>(
				'get_embedding_model_status',
				{ modelId: selectedModelId },
			);
			modelStatus = status.status;
		} catch (e) {
			console.error('Failed to check embedding model status:', e);
			error = `Failed to check model status: ${e}`;
		}
	}

	async function selectModel(modelId: string) {
		selectedModelId = modelId;
		await checkModelStatus();
	}

	async function downloadModel() {
		if (!selectedModelId) return;

		try {
			isDownloading = true;
			downloadProgress = null;
			error = null;

			unlistenDownloadProgress = await listen<DownloadProgress>(
				'embedding-model-download-progress',
				(event) => {
					downloadProgress = event.payload;
				},
			);

			unlistenDownloadComplete = await listen(
				'embedding-model-download-complete',
				async () => {
					modelStatus = 'Ready';
					downloadProgress = null;
					isDownloading = false;
					unlistenDownloadProgress?.();
					unlistenDownloadComplete?.();
				},
			);

			await invoke('download_embedding_model', { modelId: selectedModelId });
		} catch (e) {
			isDownloading = false;
			error = `Download failed: ${e}`;
			console.error('Failed to download embedding model:', e);
			unlistenDownloadProgress?.();
			unlistenDownloadComplete?.();
		}
	}

	async function configureModel() {
		if (!selectedModelId || modelStatus !== 'Ready') return;

		try {
			isConfiguring = true;
			error = null;
			await invoke('configure_embedding_model', { modelId: selectedModelId });
			currentModelId = selectedModelId;
			onModelConfigured?.(selectedModelId);
		} catch (e) {
			error = `Failed to configure model: ${e}`;
			console.error('Failed to configure embedding model:', e);
		} finally {
			isConfiguring = false;
		}
	}

	async function disableEmbeddings() {
		try {
			isConfiguring = true;
			error = null;
			await invoke('configure_embedding_model', { modelId: null });
			currentModelId = null;
			onModelConfigured?.(null);
		} catch (e) {
			error = `Failed to disable embeddings: ${e}`;
			console.error('Failed to disable embeddings:', e);
		} finally {
			isConfiguring = false;
		}
	}

	function formatBytes(bytes: number): string {
		if (bytes === 0) return '0 B';
		const k = 1024;
		const sizes = ['B', 'KB', 'MB', 'GB'];
		const i = Math.floor(Math.log(bytes) / Math.log(k));
		return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
	}

	onMount(async () => {
		isLoading = true;
		await loadAvailableModels();
		await loadCurrentModel();
		await checkModelStatus();
		isLoading = false;
	});

	onDestroy(() => {
		unlistenDownloadProgress?.();
		unlistenDownloadComplete?.();
	});
</script>

<div class="w-full">
	{#if isLoading}
		<div class="py-4 text-center text-slate-400">Loading embedding models...</div>
	{:else if isDownloading}
		<div class="text-center">
			<div class="mb-4 text-lg text-slate-300">Downloading Embedding Model</div>
			{#if downloadProgress}
				<div class="mb-2 text-sm text-slate-400">
					File {downloadProgress.file_index} of {downloadProgress.total_files}:
					{downloadProgress.file.split('/').pop()}
				</div>
				<div class="mb-2 h-2 w-full overflow-hidden rounded-full bg-slate-700">
					<div
						class="h-full bg-emerald-500 transition-all duration-300"
						style="width: {downloadProgress.overall_progress * 100}%"
					></div>
				</div>
				<div class="text-xs text-slate-500">
					{formatBytes(downloadProgress.downloaded)} / {formatBytes(
						downloadProgress.total,
					)}
					({Math.round(downloadProgress.overall_progress * 100)}% overall)
				</div>
			{:else}
				<div class="text-sm text-slate-400">Starting download...</div>
			{/if}
		</div>
	{:else}
		<div class="w-full">
			{#if showTitle}
				<div class="mb-4 text-lg text-slate-300">Embedding Model</div>
				<div class="mb-6 text-sm text-slate-400">
					Enable semantic search with an embedding model. Documents will be
					searchable by meaning, not just keywords.
				</div>
			{/if}

			{#if currentModelId}
				<div class="mb-4 flex items-center gap-2 rounded-lg border border-emerald-600 bg-emerald-900/30 px-4 py-3">
					<svg class="h-5 w-5 text-emerald-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
						<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7" />
					</svg>
					<span class="text-sm text-emerald-300">
						Semantic search enabled with {availableModels.find(m => m.id === currentModelId)?.name ?? currentModelId}
					</span>
					<button
						onclick={disableEmbeddings}
						disabled={isConfiguring}
						class="ml-auto text-xs text-slate-400 hover:text-slate-200"
					>
						Disable
					</button>
				</div>
			{:else}
				<div class="mb-4 flex items-center gap-2 rounded-lg border border-slate-600 bg-slate-800 px-4 py-3">
					<svg class="h-5 w-5 text-slate-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
						<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
					</svg>
					<span class="text-sm text-slate-400">
						Semantic search disabled. Using full-text search only.
					</span>
				</div>
			{/if}

			<div class="space-y-2">
				{#each availableModels as model (model.id)}
					<button
						onclick={() => selectModel(model.id)}
						class="w-full rounded-lg border p-3 text-left transition
							{selectedModelId === model.id
							? 'border-emerald-500 bg-emerald-900/30'
							: 'border-slate-600 hover:border-slate-500'}"
					>
						<div class="flex items-center justify-between">
							<div>
								<div class="font-medium text-slate-200">{model.name}</div>
								<div class="text-sm text-slate-400">{model.description}</div>
								<div class="mt-1 text-xs text-slate-500">
									{model.dimensions} dimensions
								</div>
							</div>
							<div class="ml-4 text-right">
								<div class="text-sm text-slate-500">{model.size_gb} GB</div>
								{#if model.id === currentModelId}
									<div class="text-xs text-emerald-400">Active</div>
								{:else if selectedModelId === model.id && modelStatus === 'Ready'}
									<div class="text-xs text-slate-400">Downloaded</div>
								{/if}
							</div>
						</div>
					</button>
				{/each}
			</div>

			<div class="mt-6 flex flex-col gap-3">
				{#if modelStatus !== 'Ready'}
					<button
						onclick={downloadModel}
						disabled={!selectedModelId}
						class="w-full rounded-md bg-emerald-600 px-4 py-2 font-medium text-white hover:bg-emerald-700 disabled:opacity-50"
					>
						Download Model
					</button>
				{:else if selectedModelId !== currentModelId}
					<button
						onclick={configureModel}
						disabled={isConfiguring}
						class="w-full rounded-md bg-emerald-600 px-4 py-2 font-medium text-white hover:bg-emerald-700 disabled:opacity-50"
					>
						{#if isConfiguring}
							<span class="inline-flex items-center gap-2">
								<svg class="h-4 w-4 animate-spin" fill="none" viewBox="0 0 24 24">
									<circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
									<path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"></path>
								</svg>
								Loading model into memory...
							</span>
						{:else}
							Enable Semantic Search
						{/if}
					</button>
					{#if isConfiguring}
						<p class="text-center text-xs text-slate-500">
							This may take 20-30 seconds on first load
						</p>
					{/if}
				{:else}
					<div class="w-full py-2 text-center text-sm text-emerald-400">
						Model active
					</div>
				{/if}
			</div>

			{#if error}
				<div class="mt-4 text-sm text-red-400">{error}</div>
			{/if}
		</div>
	{/if}
</div>
