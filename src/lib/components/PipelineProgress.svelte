<script lang="ts">
	import ProgressBar from './ProgressBar.svelte';
	import type { PipelineProgress } from '#lib/types/documents.js';

	interface Props {
		progress: PipelineProgress;
	}

	let { progress }: Props = $props();

	const stages = $derived([
		{ name: 'Store', data: progress.store },
		{ name: 'Extract', data: progress.extract },
		{ name: 'Index', data: progress.index },
	]);
</script>

<div class="grid grid-cols-3 gap-4">
	{#each stages as stage (stage.name)}
		{@const total =
			stage.data.pending +
			stage.data.active +
			stage.data.completed +
			stage.data.failed}
		{@const done = stage.data.completed + stage.data.failed}
		<ProgressBar
			label={stage.name}
			detail={`${done}/${total}`}
			value={done}
			max={total}
		/>
	{/each}
</div>
