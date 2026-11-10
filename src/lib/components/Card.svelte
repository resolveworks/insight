<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		padding?: 'sm' | 'md';
		tone?: 'default' | 'danger';
		href?: string;
		children: Snippet;
	}

	let { padding = 'md', tone = 'default', href, children }: Props = $props();

	const paddingClasses: Record<NonNullable<Props['padding']>, string> = {
		sm: 'p-4',
		md: 'p-6',
	};

	const toneClasses: Record<NonNullable<Props['tone']>, string> = {
		default: 'border-neutral-200',
		danger: 'border-red-600/30',
	};

	const interactionClasses =
		'transition-colors hover:border-blue-300 hover:shadow-sm';
</script>

{#if href}
	<!-- Callers pass already-resolved paths. -->
	<!-- eslint-disable svelte/no-navigation-without-resolve -->
	<a
		{href}
		class="block rounded-lg border bg-white {toneClasses[tone]} {paddingClasses[
			padding
		]} {interactionClasses} focus-visible:focus-ring"
	>
		{@render children()}
	</a>
	<!-- eslint-enable svelte/no-navigation-without-resolve -->
{:else}
	<div
		class="rounded-lg border bg-white {toneClasses[tone]} {paddingClasses[
			padding
		]}"
	>
		{@render children()}
	</div>
{/if}
