<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		title: string;
		description?: string;
		action?: Snippet;
		density?: 'page' | 'inline';
	}

	let { title, description, action, density = 'page' }: Props = $props();

	const page = $derived(density === 'page');
</script>

<div
	class="flex flex-col items-center justify-center text-center {page
		? 'py-12'
		: 'py-6'}"
>
	<p class="text-neutral-800">{title}</p>
	{#if description}
		<p class="mt-1 text-sm text-muted {page ? 'max-w-sm' : ''}">
			{description}
		</p>
	{/if}
	{#if action}
		<div class="mt-4">
			{@render action()}
		</div>
	{/if}
</div>
