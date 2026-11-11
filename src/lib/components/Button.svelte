<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { HTMLButtonAttributes } from 'svelte/elements';
	import LoadingSpinner from './LoadingSpinner.svelte';
	import {
		buttonClasses,
		type ButtonSize,
		type ButtonVariant,
	} from './button-classes';

	interface Props extends HTMLButtonAttributes {
		variant: ButtonVariant;
		size?: ButtonSize;
		label?: string;
		loading?: boolean;
		fullWidth?: boolean;
		children: Snippet;
	}

	let {
		variant,
		size = 'md',
		label,
		loading = false,
		fullWidth = false,
		type = 'button',
		disabled,
		children,
		...rest
	}: Props = $props();
</script>

<button
	{type}
	class="{buttonClasses(variant, size)} {fullWidth ? 'w-full' : ''}"
	disabled={disabled || loading}
	aria-busy={loading}
	aria-label={label}
	title={label}
	{...rest}
>
	{#if loading}
		<LoadingSpinner size="sm" />
	{/if}
	{@render children()}
</button>
