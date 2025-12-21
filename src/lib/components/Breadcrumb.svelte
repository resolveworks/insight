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

<nav class="flex items-center gap-2 text-sm text-slate-400">
	{#each segments as segment, i (i)}
		{#if i > 0}
			<span class="text-slate-600">/</span>
		{/if}
		{#if segment.href}
			<a
				href={resolve(segment.href ?? '')}
				class="hover:text-slate-200 transition-colors"
			>
				{segment.label}
			</a>
		{:else}
			<span class="text-slate-200">{segment.label}</span>
		{/if}
	{/each}
</nav>
