<script lang="ts">
	import type { Snippet } from 'svelte';
	import Card from './Card.svelte';
	import SectionHeading from './SectionHeading.svelte';

	interface Props {
		title: string;
		description?: string;
		footer?: Snippet;
		children?: Snippet;
	}

	let { title, description, footer, children }: Props = $props();
</script>

<Card>
	<SectionHeading {title} />
	{#if description}
		<p class="mt-1 text-sm text-neutral-500">{description}</p>
	{/if}
	{#if children}
		<div class="mt-4">
			{@render children()}
		</div>
	{/if}
	{#if footer}
		<div class="mt-4">
			{@render footer()}
		</div>
	{/if}
</Card>
