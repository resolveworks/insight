<script lang="ts">
	import {
		getModelState,
		type ModelType,
	} from '$lib/stores/model-state.svelte';

	type Props = {
		modelType: ModelType;
		title?: string;
		accentColor?: 'primary' | 'accent';
	};

	let {
		modelType,
		title = 'Downloading model',
		accentColor = 'accent',
	}: Props = $props();

	let state = $derived(getModelState(modelType));

	let progressBarClass = $derived(
		accentColor === 'primary' ? 'bg-primary-500' : 'bg-tertiary-500',
	);

	function formatBytes(bytes: number): string {
		if (bytes === 0) return '0 B';
		const k = 1024;
		const sizes = ['B', 'KB', 'MB', 'GB'];
		const i = Math.floor(Math.log(bytes) / Math.log(k));
		return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
	}
</script>

<div class="text-center">
	<h3 class="mb-4 text-lg text-neutral-700">{title}</h3>
	{#if state.progress}
		<p class="mb-2 text-sm text-neutral-500">
			File {state.progress.file_index} of {state.progress.total_files}: {state.progress.file
				.split('/')
				.pop()}
		</p>
		<div class="mb-2 h-2 overflow-hidden rounded-full bg-neutral-200">
			<div
				class="h-full transition-[width] duration-300 {progressBarClass}"
				style="width: {state.progress.overall_progress * 100}%"
			></div>
		</div>
		<p class="text-xs text-neutral-500">
			{formatBytes(state.progress.downloaded)} / {formatBytes(
				state.progress.total,
			)}
			({Math.round(state.progress.overall_progress * 100)}%)
		</p>
	{:else}
		<p class="mb-2 text-sm text-neutral-500">Starting download...</p>
		<div class="h-2 overflow-hidden rounded-full bg-neutral-200">
			<div class="h-full w-1/4 animate-pulse bg-neutral-400"></div>
		</div>
	{/if}
</div>
