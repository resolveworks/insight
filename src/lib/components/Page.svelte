<script lang="ts">
	import type { Snippet } from 'svelte';

	type Measure = 'wide' | 'prose';

	interface Props {
		header: Snippet;
		children: Snippet;
		measure?: Measure;
	}

	let { header, children, measure = 'wide' }: Props = $props();

	const measureClasses = $derived(
		measure === 'prose' ? 'mx-auto max-w-2xl' : '',
	);
</script>

<div class="flex h-full flex-col bg-neutral-50">
	{@render header()}
	<div class="min-h-0 flex-1 overflow-y-auto p-6">
		<div class="flex flex-col gap-6 {measureClasses}">
			{@render children()}
		</div>
	</div>
</div>
