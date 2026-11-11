<script lang="ts">
	import type { Snippet } from 'svelte';
	import LoadingSpinner from './LoadingSpinner.svelte';
	import {
		buttonClasses,
		type ButtonSize,
		type ButtonVariant,
	} from './button-classes';

	interface Props {
		variant: ButtonVariant;
		size?: ButtonSize;
		/** Accessible name for icon-only buttons. */
		label?: string;
		type?: 'button' | 'submit';
		disabled?: boolean;
		loading?: boolean;
		fullWidth?: boolean;
		/** A Modal focuses this control when it opens. */
		autofocus?: boolean;
		onclick?: (event: MouseEvent) => void;
		children: Snippet;
	}

	let {
		variant,
		size = 'md',
		label,
		type = 'button',
		disabled = false,
		loading = false,
		fullWidth = false,
		autofocus = false,
		onclick,
		children,
	}: Props = $props();
</script>

<button
	{type}
	class="{buttonClasses(variant, size)} {fullWidth ? 'w-full' : ''}"
	data-autofocus={autofocus ? '' : undefined}
	disabled={disabled || loading}
	aria-busy={loading}
	aria-label={label}
	title={label}
	{onclick}
>
	{#if loading}
		<LoadingSpinner size="sm" />
	{/if}
	{@render children()}
</button>
