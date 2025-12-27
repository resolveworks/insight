<script lang="ts">
	import { resolve } from '$app/paths';

	type Segment = {
		label: string;
		href?: string;
	};

	type Props = {
		segments: Segment[];
	};

	let { segments }: Props = $props();
</script>

<nav class="flex items-center gap-2 text-sm text-neutral-400">
	{#each segments as segment, i (i)}
		{#if i > 0}
			<span class="text-neutral-600">/</span>
		{/if}
		{#if segment.href}
			{@const href = resolve(segment.href as '/')}
			<a {href} class="hover:text-neutral-200 transition-colors">
				{segment.label}
			</a>
		{:else}
			<span class="text-neutral-200">{segment.label}</span>
		{/if}
	{/each}
</nav>
