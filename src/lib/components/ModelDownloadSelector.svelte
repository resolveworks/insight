<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	import { onDestroy, onMount } from 'svelte';

	export interface ModelInfo {
		id: string;
		name: string;
		description: string;
		size_gb: number;
		dimensions?: number;
	}

	export interface DownloadProgress {
		file: string;
		downloaded: number;
		total: number;
		overall_progress: number;
		file_index: number;
		total_files: number;
	}

	type ModelType = 'llm' | 'embedding';
	type Status = 'loading' | 'idle' | 'downloading' | 'configuring';

	type Props = {
		modelType: ModelType;
		onReady?: (modelId: string | null) => void;
		showTitle?: boolean;
	};

	let { modelType, onReady, showTitle = true }: Props = $props();

	// Command/event names based on model type
	const commands = $derived(
		modelType === 'llm'
			? {
					list: 'get_available_models',
					status: 'get_model_status',
					download: 'download_model',
					current: null,
					configure: null,
					progressEvent: 'model-download-progress',
					completeEvent: 'model-download-complete',
				}
			: {
					list: 'get_available_embedding_models',
					status: 'get_embedding_model_status',
					download: 'download_embedding_model',
					current: 'get_current_embedding_model',
					configure: 'configure_embedding_model',
					progressEvent: 'embedding-model-download-progress',
					completeEvent: 'embedding-model-download-complete',
				},
	);

	let models = $state<ModelInfo[]>([]);
	let selectedId = $state<string | null>(null);
	let activeId = $state<string | null>(null);
	let isDownloaded = $state(false);
	let status = $state<Status>('loading');
	let progress = $state<DownloadProgress | null>(null);
	let error = $state<string | null>(null);

	let unlistenProgress: UnlistenFn | undefined;
	let unlistenComplete: UnlistenFn | undefined;

	// Derived state
	let selectedModel = $derived(models.find((m) => m.id === selectedId));
	let isEmbedding = $derived(modelType === 'embedding');
	let canDownload = $derived(selectedId && !isDownloaded && status === 'idle');
	let canConfigure = $derived(isEmbedding && isDownloaded && selectedId !== activeId && status === 'idle');
	let isActive = $derived(isEmbedding && selectedId === activeId);

	// Public API
	export function getSelectedModelId(): string | null {
		return selectedId;
	}

	export function isReady(): boolean {
		return isDownloaded;
	}

	async function load() {
		status = 'loading';
		error = null;

		try {
			models = await invoke<ModelInfo[]>(commands.list);
			if (models.length > 0) {
				// Check for active model first (embedding only)
				if (commands.current) {
					activeId = await invoke<string | null>(commands.current);
				}
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
			const result = await invoke<{ status: string }>(commands.status, { modelId: selectedId });
			isDownloaded = result.status === 'Ready';

			// For LLM, notify ready immediately when downloaded
			if (!isEmbedding && isDownloaded) {
				onReady?.(selectedId);
			}
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

		status = 'downloading';
		progress = null;
		error = null;

		try {
			unlistenProgress = await listen<DownloadProgress>(commands.progressEvent, (e) => {
				progress = e.payload;
			});

			unlistenComplete = await listen(commands.completeEvent, () => {
				isDownloaded = true;
				progress = null;
				status = 'idle';
				cleanup();

				if (!isEmbedding && selectedId) {
					onReady?.(selectedId);
				}
			});

			await invoke(commands.download, { modelId: selectedId });
		} catch (e) {
			error = `Download failed: ${e}`;
			status = 'idle';
			cleanup();
		}
	}

	async function configure() {
		if (!commands.configure || !selectedId) return;

		status = 'configuring';
		error = null;

		try {
			await invoke(commands.configure, { modelId: selectedId });
			activeId = selectedId;
			onReady?.(selectedId);
		} catch (e) {
			error = `Failed to configure: ${e}`;
		} finally {
			status = 'idle';
		}
	}

	async function disable() {
		if (!commands.configure) return;

		status = 'configuring';
		error = null;

		try {
			await invoke(commands.configure, { modelId: null });
			activeId = null;
			onReady?.(null);
		} catch (e) {
			error = `Failed to disable: ${e}`;
		} finally {
			status = 'idle';
		}
	}

	function cleanup() {
		unlistenProgress?.();
		unlistenComplete?.();
	}

	function formatBytes(bytes: number): string {
		if (bytes === 0) return '0 B';
		const k = 1024;
		const sizes = ['B', 'KB', 'MB', 'GB'];
		const i = Math.floor(Math.log(bytes) / Math.log(k));
		return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
	}

	onMount(load);
	onDestroy(cleanup);
</script>

<div class="model-selector" class:embedding={isEmbedding}>
	{#if status === 'loading'}
		<p class="status-text">Loading models...</p>
	{:else if status === 'downloading'}
		<div class="download-progress">
			<h3>Downloading {isEmbedding ? 'Embedding Model' : 'Model'}</h3>
			{#if progress}
				<p class="file-info">
					File {progress.file_index} of {progress.total_files}: {progress.file.split('/').pop()}
				</p>
				<div class="progress-bar">
					<div class="progress-fill" style="width: {progress.overall_progress * 100}%"></div>
				</div>
				<p class="progress-text">
					{formatBytes(progress.downloaded)} / {formatBytes(progress.total)}
					({Math.round(progress.overall_progress * 100)}%)
				</p>
			{:else}
				<p class="file-info">Starting download...</p>
			{/if}
		</div>
	{:else}
		{#if showTitle}
			<h3>{isEmbedding ? 'Embedding Model' : 'Select AI Model'}</h3>
			<p class="description">
				{#if isEmbedding}
					Enable semantic search to find documents by meaning, not just keywords.
				{:else}
					Choose a model to download. Smaller models are faster but less capable.
				{/if}
			</p>
		{/if}

		{#if isEmbedding}
			<div class="status-banner" class:active={activeId}>
				{#if activeId}
					<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor">
						<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7" />
					</svg>
					<span>Semantic search enabled with {models.find((m) => m.id === activeId)?.name}</span>
					<button class="disable-btn" onclick={disable} disabled={status === 'configuring'}>
						Disable
					</button>
				{:else}
					<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor">
						<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
					</svg>
					<span>Semantic search disabled. Using full-text search only.</span>
				{/if}
			</div>
		{/if}

		<div class="model-list">
			{#each models as model (model.id)}
				<button class="model-card" class:selected={selectedId === model.id} onclick={() => select(model.id)}>
					<div class="model-info">
						<span class="model-name">{model.name}</span>
						<span class="model-desc">{model.description}</span>
						{#if isEmbedding && model.dimensions}
							<span class="model-dims">{model.dimensions} dimensions</span>
						{/if}
					</div>
					<div class="model-meta">
						<span class="model-size">{model.size_gb} GB</span>
						{#if model.id === activeId}
							<span class="badge active">Active</span>
						{:else if model.id === selectedId && isDownloaded}
							<span class="badge downloaded">Downloaded</span>
						{/if}
					</div>
				</button>
			{/each}
		</div>

		<div class="actions">
			{#if canDownload}
				<button class="btn primary" onclick={download}>
					Download {isEmbedding ? 'Model' : 'Selected Model'}
				</button>
			{:else if canConfigure}
				<button class="btn primary" onclick={configure} disabled={status === 'configuring'}>
					{#if status === 'configuring'}
						<svg class="spinner" viewBox="0 0 24 24" fill="none">
							<circle class="spinner-track" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
							<path class="spinner-fill" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"></path>
						</svg>
						Loading model...
					{:else}
						Enable Semantic Search
					{/if}
				</button>
				{#if status === 'configuring'}
					<p class="hint">This may take 20-30 seconds on first load</p>
				{/if}
			{:else if isActive}
				<p class="ready-text">Model active</p>
			{:else if isDownloaded}
				<p class="ready-text">Model ready to use</p>
			{/if}
		</div>

		{#if error}
			<p class="error">{error}</p>
		{/if}
	{/if}
</div>

<style>
	.model-selector {
		--accent: theme('colors.rose.500');
		--accent-bg: theme('colors.rose.900' / 30%);
		--accent-hover: theme('colors.rose.700');
	}

	.model-selector.embedding {
		--accent: theme('colors.emerald.500');
		--accent-bg: theme('colors.emerald.900' / 30%);
		--accent-hover: theme('colors.emerald.700');
	}

	h3 {
		font-size: theme('fontSize.lg');
		color: theme('colors.slate.300');
		margin-bottom: theme('spacing.4');
	}

	.description {
		font-size: theme('fontSize.sm');
		color: theme('colors.slate.400');
		margin-bottom: theme('spacing.6');
	}

	.status-text {
		color: theme('colors.slate.400');
		text-align: center;
		padding: theme('spacing.4') 0;
	}

	/* Status banner (embedding only) */
	.status-banner {
		display: flex;
		align-items: center;
		gap: theme('spacing.2');
		padding: theme('spacing.3') theme('spacing.4');
		border-radius: theme('borderRadius.lg');
		border: 1px solid theme('colors.slate.600');
		background: theme('colors.slate.800');
		margin-bottom: theme('spacing.4');
		font-size: theme('fontSize.sm');
		color: theme('colors.slate.400');
	}

	.status-banner.active {
		border-color: var(--accent);
		background: var(--accent-bg);
		color: theme('colors.emerald.300');
	}

	.status-banner .icon {
		width: theme('spacing.5');
		height: theme('spacing.5');
		flex-shrink: 0;
	}

	.status-banner.active .icon {
		color: theme('colors.emerald.400');
	}

	.disable-btn {
		margin-left: auto;
		font-size: theme('fontSize.xs');
		color: theme('colors.slate.400');
		cursor: pointer;
	}

	.disable-btn:hover {
		color: theme('colors.slate.200');
	}

	/* Model list */
	.model-list {
		display: flex;
		flex-direction: column;
		gap: theme('spacing.2');
	}

	.model-card {
		display: flex;
		justify-content: space-between;
		align-items: center;
		width: 100%;
		padding: theme('spacing.3');
		border-radius: theme('borderRadius.lg');
		border: 1px solid theme('colors.slate.600');
		background: transparent;
		text-align: left;
		cursor: pointer;
		transition: border-color 0.15s;
	}

	.model-card:hover {
		border-color: theme('colors.slate.500');
	}

	.model-card.selected {
		border-color: var(--accent);
		background: var(--accent-bg);
	}

	.model-info {
		display: flex;
		flex-direction: column;
		gap: theme('spacing.0.5');
	}

	.model-name {
		font-weight: 500;
		color: theme('colors.slate.200');
	}

	.model-desc {
		font-size: theme('fontSize.sm');
		color: theme('colors.slate.400');
	}

	.model-dims {
		font-size: theme('fontSize.xs');
		color: theme('colors.slate.500');
		margin-top: theme('spacing.1');
	}

	.model-meta {
		display: flex;
		flex-direction: column;
		align-items: flex-end;
		gap: theme('spacing.1');
		margin-left: theme('spacing.4');
	}

	.model-size {
		font-size: theme('fontSize.sm');
		color: theme('colors.slate.500');
	}

	.badge {
		font-size: theme('fontSize.xs');
	}

	.badge.active {
		color: theme('colors.emerald.400');
	}

	.badge.downloaded {
		color: theme('colors.green.400');
	}

	.embedding .badge.downloaded {
		color: theme('colors.slate.400');
	}

	/* Actions */
	.actions {
		margin-top: theme('spacing.6');
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: theme('spacing.3');
	}

	.btn {
		padding: theme('spacing.2') theme('spacing.4');
		border-radius: theme('borderRadius.md');
		font-weight: 500;
		cursor: pointer;
		transition: background-color 0.15s;
	}

	.btn.primary {
		background: var(--accent);
		color: white;
		width: 100%;
	}

	.btn.primary:hover:not(:disabled) {
		background: var(--accent-hover);
	}

	.btn:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}

	.spinner {
		width: theme('spacing.4');
		height: theme('spacing.4');
		display: inline-block;
		margin-right: theme('spacing.2');
		animation: spin 1s linear infinite;
	}

	.spinner-track {
		opacity: 0.25;
	}

	@keyframes spin {
		to {
			transform: rotate(360deg);
		}
	}

	.hint {
		font-size: theme('fontSize.xs');
		color: theme('colors.slate.500');
		text-align: center;
	}

	.ready-text {
		font-size: theme('fontSize.sm');
		color: theme('colors.green.400');
	}

	.embedding .ready-text {
		color: theme('colors.emerald.400');
	}

	.error {
		margin-top: theme('spacing.4');
		font-size: theme('fontSize.sm');
		color: theme('colors.red.400');
	}

	/* Download progress */
	.download-progress {
		text-align: center;
	}

	.download-progress h3 {
		margin-bottom: theme('spacing.4');
	}

	.file-info {
		font-size: theme('fontSize.sm');
		color: theme('colors.slate.400');
		margin-bottom: theme('spacing.2');
	}

	.progress-bar {
		height: theme('spacing.2');
		background: theme('colors.slate.700');
		border-radius: theme('borderRadius.full');
		overflow: hidden;
		margin-bottom: theme('spacing.2');
	}

	.progress-fill {
		height: 100%;
		background: var(--accent);
		transition: width 0.3s;
	}

	.progress-text {
		font-size: theme('fontSize.xs');
		color: theme('colors.slate.500');
	}
</style>
