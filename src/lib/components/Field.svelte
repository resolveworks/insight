<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		id?: string | null;
		label?: string;
		hint?: string;
		class?: string;
		children: Snippet;
	}

	let { id, label, hint, class: className = '', children }: Props = $props();
</script>

<div class="flex flex-col gap-2 {className}">
	{#if label}
		<label for={id} class="text-sm font-medium text-neutral-800">{label}</label>
	{/if}
	{@render children()}
	{#if hint}
		<p class="text-xs text-muted">{hint}</p>
	{/if}
</div>
