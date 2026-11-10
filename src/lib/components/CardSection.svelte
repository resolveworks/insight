<script lang="ts">
	import type { Snippet } from 'svelte';
	import Card from './Card.svelte';
	import SectionHeading from './SectionHeading.svelte';

	interface Props {
		title: string;
		description?: string;
		action?: Snippet;
		footer?: Snippet;
		tone?: 'default' | 'danger';
		children: Snippet;
	}

	let {
		title,
		description,
		action,
		footer,
		tone = 'default',
		children,
	}: Props = $props();
</script>

<Card {tone}>
	<div class="flex items-start justify-between gap-4">
		<SectionHeading {title} />
		{#if action}
			<div class="shrink-0">
				{@render action()}
			</div>
		{/if}
	</div>
	{#if description}
		<p class="mt-1 text-sm text-muted">{description}</p>
	{/if}
	<div class="mt-4">
		{@render children()}
	</div>
	{#if footer}
		<div class="mt-4">
			{@render footer()}
		</div>
	{/if}
</Card>
