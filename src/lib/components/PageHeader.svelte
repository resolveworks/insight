<script lang="ts">
	import type { Snippet } from 'svelte';
	import ButtonLink from './ButtonLink.svelte';
	import Icon from './Icon.svelte';

	interface Props {
		back?: { href: string; label: string };
		title: string;
		subtitle?: Snippet;
		actions?: Snippet;
		tabs?: Snippet;
	}

	let { back, title, subtitle, actions, tabs }: Props = $props();
</script>

<header class="border-b border-neutral-200 bg-white px-6 py-4">
	<div class="flex items-center justify-between gap-4">
		<div class="min-w-0">
			{#if back}
				<div class="mb-1">
					<ButtonLink href={back.href} variant="outline" size="sm">
						<Icon name="arrow-left" size="sm" />
						{back.label}
					</ButtonLink>
				</div>
			{/if}
			<h1 class="truncate text-xl font-semibold text-neutral-800">{title}</h1>
			{#if subtitle}
				<div class="mt-1">
					{@render subtitle()}
				</div>
			{/if}
		</div>
		{#if actions}
			<div class="shrink-0">
				{@render actions()}
			</div>
		{/if}
	</div>
	{#if tabs}
		<div class="mt-3">
			{@render tabs()}
		</div>
	{/if}
</header>
