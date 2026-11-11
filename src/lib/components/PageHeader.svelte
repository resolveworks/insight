<script lang="ts">
	import type { Snippet } from 'svelte';
	import ButtonLink from './ButtonLink.svelte';
	import Icon from './Icon.svelte';

	interface Props {
		back?: { href: string; label: string };
		title: string;
		actions?: Snippet;
	}

	let { back, title, actions }: Props = $props();
</script>

<header class="border-b border-neutral-200 bg-white px-6 py-4">
	{#if back}
		<ButtonLink href={back.href} variant="outline" size="sm">
			<Icon name="arrow-left" size="sm" />
			{back.label}
		</ButtonLink>
	{/if}
	<div class="flex items-center justify-between gap-4">
		<h1 class="min-w-0 truncate text-xl font-semibold text-neutral-800">
			{title}
		</h1>
		{#if actions}
			<div class="max-w-full shrink-0 overflow-x-auto">
				{@render actions()}
			</div>
		{/if}
	</div>
</header>
