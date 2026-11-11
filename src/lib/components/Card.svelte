<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		padding?: 'sm' | 'md';
		href?: string;
		children: Snippet;
	}

	let { padding = 'md', href, children }: Props = $props();

	const paddingClasses: Record<NonNullable<Props['padding']>, string> = {
		sm: 'p-4',
		md: 'p-6',
	};

	const interactionClasses =
		'transition-colors hover:border-neutral-400 hover:shadow-sm';
</script>

{#if href}
	<!-- Callers pass already-resolved paths. -->
	<!-- eslint-disable svelte/no-navigation-without-resolve -->
	<a
		{href}
		class="block rounded-lg border border-neutral-200 bg-white {paddingClasses[
			padding
		]} {interactionClasses} focus-visible:focus-ring"
	>
		{@render children()}
	</a>
	<!-- eslint-enable svelte/no-navigation-without-resolve -->
{:else}
	<div
		class="rounded-lg border border-neutral-200 bg-white {paddingClasses[
			padding
		]}"
	>
		{@render children()}
	</div>
{/if}
