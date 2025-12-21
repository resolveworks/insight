<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	import { onDestroy, onMount } from 'svelte';

	export interface ModelInfo {
		id: string;
		name: string;
		description: string;
		size_gb: number;
	}

	export interface ModelStatus {
		status: 'NotDownloaded' | 'Downloading' | 'Ready' | 'Failed';
		path?: string;
		progress?: DownloadProgress;
		error?: string;
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
		onModelReady?: (modelId: string) => void;
		showTitle?: boolean;
	};

	let { onModelReady, showTitle = true }: Props = $props();

	let availableModels = $state<ModelInfo[]>([]);
	let selectedModelId = $state<string | null>(null);
	let modelStatus = $state<ModelStatus['status']>('NotDownloaded');
	let downloadProgress = $state<DownloadProgress | null>(null);
	let isCheckingModel = $state(true);
	let error = $state<string | null>(null);

	let unlistenDownloadProgress: UnlistenFn | undefined;
	let unlistenDownloadComplete: UnlistenFn | undefined;

	export function getSelectedModelId(): string | null {
		return selectedModelId;
	}

	export function getModelStatus(): ModelStatus['status'] {
		return modelStatus;
	}

	export function isReady(): boolean {
		return modelStatus === 'Ready';
	}

	async function loadAvailableModels() {
		try {
			availableModels = await invoke<ModelInfo[]>('get_available_models');
			if (availableModels.length > 0 && !selectedModelId) {
				selectedModelId = availableModels[0].id;
			}
		} catch (e) {
			console.error('Failed to load available models:', e);
			error = `Failed to load models: ${e}`;
		}
	}

	async function checkModelStatus() {
		if (!selectedModelId) return;
		try {
			isCheckingModel = true;
			const status = await invoke<ModelStatus>('get_model_status', {
				modelId: selectedModelId,
			});
			modelStatus = status.status;
			if (modelStatus === 'Ready') {
				onModelReady?.(selectedModelId);
			}
		} catch (e) {
			console.error('Failed to check model status:', e);
			error = `Failed to check model status: ${e}`;
		} finally {
			isCheckingModel = false;
		}
	}

	async function selectModel(modelId: string) {
		selectedModelId = modelId;
		await checkModelStatus();
	}

	async function downloadModel() {
		if (!selectedModelId) return;

		try {
			modelStatus = 'Downloading';
			downloadProgress = null;
			error = null;

			unlistenDownloadProgress = await listen<DownloadProgress>(
				'model-download-progress',
				(event) => {
					downloadProgress = event.payload;
				},
			);

			unlistenDownloadComplete = await listen(
				'model-download-complete',
				async () => {
					modelStatus = 'Ready';
					downloadProgress = null;
					unlistenDownloadProgress?.();
					unlistenDownloadComplete?.();
					if (selectedModelId) {
						onModelReady?.(selectedModelId);
					}
				},
			);

			await invoke('download_model', { modelId: selectedModelId });
		} catch (e) {
			modelStatus = 'Failed';
			error = `Download failed: ${e}`;
			console.error('Failed to download model:', e);
			unlistenDownloadProgress?.();
			unlistenDownloadComplete?.();
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
		await loadAvailableModels();
		await checkModelStatus();
	});

	onDestroy(() => {
		unlistenDownloadProgress?.();
		unlistenDownloadComplete?.();
	});
</script>

<div class="flex h-full items-center justify-center">
	{#if isCheckingModel}
		<div class="text-center text-slate-400">
			<div class="mb-2 text-lg">Checking model status...</div>
		</div>
	{:else if modelStatus === 'Downloading'}
		<div class="w-full max-w-md text-center">
			<div class="mb-4 text-lg text-slate-300">Downloading Model</div>
			{#if downloadProgress}
				<div class="mb-2 text-sm text-slate-400">
					File {downloadProgress.file_index} of {downloadProgress.total_files}:
					{downloadProgress.file.split('/').pop()}
				</div>
				<div class="mb-2 h-2 w-full overflow-hidden rounded-full bg-slate-700">
					<div
						class="h-full bg-rose-500 transition-all duration-300"
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
		<div class="w-full max-w-md px-4 text-center">
			{#if showTitle}
				<div class="mb-4 text-lg text-slate-300">Select AI Model</div>
				<div class="mb-6 text-sm text-slate-400">
					Choose a model to download. Smaller models are faster but less
					capable.
				</div>
			{/if}

			<div class="mb-6 space-y-2">
				{#each availableModels as model (model.id)}
					<button
						onclick={() => selectModel(model.id)}
						class="w-full rounded-lg border p-3 text-left transition
							{selectedModelId === model.id
							? 'border-rose-500 bg-rose-900/30'
							: 'border-slate-600 hover:border-slate-500'}"
					>
						<div class="flex items-center justify-between">
							<div>
								<div class="font-medium text-slate-200">{model.name}</div>
								<div class="text-sm text-slate-400">{model.description}</div>
							</div>
							<div class="ml-4 text-right">
								<div class="text-sm text-slate-500">{model.size_gb} GB</div>
								{#if selectedModelId === model.id && modelStatus === 'Ready'}
									<div class="text-xs text-green-400">Downloaded</div>
								{/if}
							</div>
						</div>
					</button>
				{/each}
			</div>

			{#if modelStatus !== 'Ready'}
				<button
					onclick={downloadModel}
					disabled={!selectedModelId}
					class="rounded-md bg-rose-600 px-6 py-3 font-medium text-white hover:bg-rose-700 disabled:opacity-50"
				>
					Download Selected Model
				</button>
				{#if modelStatus === 'Failed'}
					<div class="mt-4 text-sm text-red-400">
						Previous download failed. Click to retry.
					</div>
				{/if}
			{:else}
				<div class="text-sm text-green-400">Model ready to use</div>
			{/if}

			{#if error}
				<div class="mt-4 text-sm text-red-400">{error}</div>
			{/if}
		</div>
	{/if}
</div>
