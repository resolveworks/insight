<script lang="ts">
	import type { Snippet } from 'svelte';

	type Measure = 'wide' | 'prose';

	interface Props {
		header?: Snippet;
		children: Snippet;
		measure?: Measure;
		fill?: boolean;
	}

	let { header, children, measure = 'wide', fill = false }: Props = $props();

	const measureClasses = $derived(
		measure === 'prose' ? 'mx-auto max-w-2xl' : '',
	);
</script>

<div class="flex h-full flex-col bg-neutral-50">
	{#if header}
		{@render header()}
	{/if}
	<div class="min-h-0 flex-1 overflow-y-auto p-6">
		<div class="flex flex-col gap-6 {measureClasses} {fill ? 'h-full' : ''}">
			{@render children()}
		</div>
	</div>
</div>
