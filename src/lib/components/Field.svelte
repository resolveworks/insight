<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		id?: string | null;
		label?: string;
		hint?: string;
		children: Snippet;
	}

	let { id, label, hint, children }: Props = $props();
</script>

<div class="flex flex-col gap-2">
	{#if label}
		<label for={id} class="text-sm font-medium text-neutral-800">{label}</label>
	{/if}
	{@render children()}
	{#if hint}
		<p class="text-xs text-neutral-500">{hint}</p>
	{/if}
</div>
