<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		title: string;
		description?: string;
		action?: Snippet;
	}

	let { title, description, action }: Props = $props();
</script>

<div class="flex flex-col items-center justify-center py-12 text-center">
	<p class="text-neutral-800">{title}</p>
	{#if description}
		<p class="mt-1 max-w-sm text-sm text-neutral-500">
			{description}
		</p>
	{/if}
	{#if action}
		<div class="mt-4">
			{@render action()}
		</div>
	{/if}
</div>
