<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { listen } from '@tauri-apps/api/event';
	import { onMount } from 'svelte';
	import { embeddingModelConfig } from '$lib/models/config';

	type Props = {
		modelId: string;
		modelName: string;
		onComplete: () => void;
	};

	let { modelId, modelName, onComplete }: Props = $props();

	type DownloadProgress = {
		file: string;
		downloaded: number;
		total: number;
		overall_progress: number;
		file_index: number;
		total_files: number;
	};

	let status = $state<'idle' | 'downloading' | 'configuring' | 'error'>('idle');
	let progress = $state<DownloadProgress | null>(null);
	let error = $state<string | null>(null);

	function formatBytes(bytes: number): string {
		if (bytes === 0) return '0 B';
		const k = 1024;
		const sizes = ['B', 'KB', 'MB', 'GB'];
		const i = Math.floor(Math.log(bytes) / Math.log(k));
		return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
	}

	async function startDownload() {
		status = 'downloading';
		error = null;

		try {
			// Listen for progress events
			const unlistenProgress = await listen<DownloadProgress>(
				embeddingModelConfig.progressEvent,
				(event) => {
					progress = event.payload;
				},
			);

			// Listen for completion
			const unlistenComplete = await listen(
				embeddingModelConfig.completeEvent,
				async () => {
					unlistenProgress();
					unlistenComplete();

					// Configure the model after download
					status = 'configuring';
					try {
						await invoke(embeddingModelConfig.configureCommand, {
							modelId: modelId,
						});
						onComplete();
					} catch (e) {
						error = `Failed to configure model: ${e}`;
						status = 'error';
					}
				},
			);

			// Start download
			await invoke(embeddingModelConfig.downloadCommand, { modelId });
		} catch (e) {
			error = `Download failed: ${e}`;
			status = 'error';
		}
	}

	onMount(() => {
		// Auto-start download
		startDownload();
	});
</script>

<div class="flex h-screen flex-col items-center justify-center bg-slate-900">
	<div class="w-96 text-center">
		{#if status === 'downloading'}
			<!-- Download in progress -->
			<div class="mb-6">
				<svg
					class="mx-auto h-12 w-12 text-emerald-500"
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

			<h1 class="mb-2 text-xl font-semibold text-slate-100">
				Downloading {modelName}
			</h1>

			<p class="mb-6 text-sm text-slate-400">
				This is a one-time download. The model will be cached locally.
			</p>

			{#if progress}
				<div class="mb-3">
					<p class="mb-2 text-sm text-slate-400">
						File {progress.file_index} of {progress.total_files}: {progress.file
							.split('/')
							.pop()}
					</p>
					<div class="h-2 overflow-hidden rounded-full bg-slate-700">
						<div
							class="h-full bg-emerald-500 transition-[width] duration-300"
							style="width: {progress.overall_progress * 100}%"
						></div>
					</div>
					<p class="mt-2 text-xs text-slate-500">
						{formatBytes(progress.downloaded)} / {formatBytes(progress.total)}
						({Math.round(progress.overall_progress * 100)}%)
					</p>
				</div>
			{:else}
				<div class="mb-3">
					<p class="mb-2 text-sm text-slate-400">Starting download...</p>
					<div class="h-2 overflow-hidden rounded-full bg-slate-700">
						<div class="h-full w-1/4 animate-pulse bg-emerald-500/50"></div>
					</div>
				</div>
			{/if}
		{:else if status === 'configuring'}
			<!-- Configuring after download -->
			<div class="mb-6">
				<svg
					class="mx-auto h-12 w-12 animate-spin text-emerald-500"
					fill="none"
					viewBox="0 0 24 24"
				>
					<circle
						class="opacity-25"
						cx="12"
						cy="12"
						r="10"
						stroke="currentColor"
						stroke-width="4"
					></circle>
					<path
						class="opacity-75"
						fill="currentColor"
						d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
					></path>
				</svg>
			</div>

			<h1 class="mb-2 text-xl font-semibold text-slate-100">
				Loading {modelName}
			</h1>

			<p class="text-sm text-slate-400">
				This may take 20-30 seconds on first load...
			</p>
		{:else if status === 'error'}
			<!-- Error state -->
			<div class="mb-6">
				<svg
					class="mx-auto h-12 w-12 text-red-500"
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
			</div>

			<h1 class="mb-2 text-xl font-semibold text-red-400">Download Failed</h1>

			<p class="mb-6 text-sm text-slate-400">{error}</p>

			<button
				onclick={startDownload}
				class="rounded-md bg-emerald-600 px-6 py-2 font-medium text-white transition-colors hover:bg-emerald-700"
			>
				Retry Download
			</button>
		{:else}
			<!-- Idle - should auto-start but show button just in case -->
			<div class="mb-6">
				<svg
					class="mx-auto h-12 w-12 text-slate-500"
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

			<h1 class="mb-2 text-xl font-semibold text-slate-100">
				Download Required
			</h1>

			<p class="mb-6 text-sm text-slate-400">
				The embedding model "{modelName}" needs to be downloaded before Insight
				can start.
			</p>

			<button
				onclick={startDownload}
				class="rounded-md bg-emerald-600 px-6 py-2 font-medium text-white transition-colors hover:bg-emerald-700"
			>
				Download Model
			</button>
		{/if}
	</div>
</div>
