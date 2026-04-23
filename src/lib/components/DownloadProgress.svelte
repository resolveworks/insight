<script lang="ts">
	import {
		getProviderState,
		type ProviderType,
	} from '$lib/stores/provider-state.svelte';

	type Props = {
		providerType: ProviderType;
		title?: string;
		accentColor?: 'primary' | 'accent';
	};

	let {
		providerType,
		title = 'Downloading model',
		accentColor = 'accent',
	}: Props = $props();

	const state = $derived(getProviderState(providerType));
	const progress = $derived(
		state.status.kind === 'downloading' ? state.status.progress : null,
	);

	const progressBarClass = $derived(
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
	{#if progress}
		<p class="mb-2 text-sm text-neutral-500">
			File {progress.file_index} of {progress.total_files}: {progress.file
				.split('/')
				.pop()}
		</p>
		<div class="mb-2 h-2 overflow-hidden rounded-full bg-neutral-200">
			<div
				class="h-full transition-[width] duration-300 {progressBarClass}"
				style="width: {progress.overall_progress * 100}%"
			></div>
		</div>
		<p class="text-xs text-neutral-500">
			{formatBytes(progress.downloaded)} / {formatBytes(progress.total)}
			({Math.round(progress.overall_progress * 100)}%)
		</p>
	{:else}
		<p class="mb-2 text-sm text-neutral-500">Starting download...</p>
		<div class="h-2 overflow-hidden rounded-full bg-neutral-200">
			<div class="h-full w-1/4 animate-pulse bg-neutral-400"></div>
		</div>
	{/if}
</div>
