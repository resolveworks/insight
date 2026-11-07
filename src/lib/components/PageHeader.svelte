<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		eyebrow?: string;
		title: string;
		actions?: Snippet;
		tabs?: Snippet;
	}

	let { eyebrow, title, actions, tabs }: Props = $props();
</script>

<header class="border-b border-line bg-surface-bright px-6 py-4">
	<div class="flex items-center justify-between gap-4">
		<div class="min-w-0">
			{#if eyebrow}
				<p class="text-xs uppercase tracking-wide text-subtle">{eyebrow}</p>
			{/if}
			<h1 class="truncate text-xl font-medium text-neutral-800">{title}</h1>
		</div>
		{#if actions}
			<div class="shrink-0">
				{@render actions()}
			</div>
		{/if}
	</div>
	{#if tabs}
		<div class="mt-3">
			{@render tabs()}
		</div>
	{/if}
</header>
