<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		title: string;
		level?: 2 | 3;
		id?: string;
		action?: Snippet;
		class?: string;
	}

	let { title, level = 2, id, action, class: className = '' }: Props = $props();
</script>

{#if action}
	<div class="flex items-center justify-between gap-4 {className}">
		<svelte:element
			this={`h${level}`}
			{id}
			class="text-sm font-semibold text-neutral-800"
		>
			{title}
		</svelte:element>
		<div class="shrink-0">
			{@render action()}
		</div>
	</div>
{:else}
	<svelte:element
		this={`h${level}`}
		{id}
		class="text-sm font-semibold text-neutral-800 {className}"
	>
		{title}
	</svelte:element>
{/if}
