<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		padding?: 'none' | 'sm' | 'md';
		tone?: 'default' | 'danger';
		interactive?: boolean;
		href?: string;
		children: Snippet;
	}

	let {
		padding = 'md',
		tone = 'default',
		interactive = false,
		href,
		children,
	}: Props = $props();

	const paddingClasses: Record<NonNullable<Props['padding']>, string> = {
		none: '',
		sm: 'p-4',
		md: 'p-6',
	};

	const toneClasses: Record<NonNullable<Props['tone']>, string> = {
		default: 'border-line',
		danger: 'border-error/30',
	};

	const interactionClasses =
		'transition-colors hover:border-primary-300 hover:shadow-sm';
</script>

{#if href}
	<!-- Callers pass already-resolved paths. -->
	<!-- eslint-disable svelte/no-navigation-without-resolve -->
	<a
		{href}
		class="block rounded-lg border bg-surface-bright {toneClasses[
			tone
		]} {paddingClasses[padding]} {interactionClasses}"
	>
		{@render children()}
	</a>
	<!-- eslint-enable svelte/no-navigation-without-resolve -->
{:else}
	<div
		class="rounded-lg border bg-surface-bright {toneClasses[
			tone
		]} {paddingClasses[padding]} {interactive ? interactionClasses : ''}"
	>
		{@render children()}
	</div>
{/if}
