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
		variant?: ButtonVariant;
		size?: ButtonSize;
		loading?: boolean;
		fullWidth?: boolean;
		children: Snippet;
	}

	let {
		variant = 'primary',
		size = 'md',
		loading = false,
		fullWidth = false,
		type = 'button',
		disabled,
		class: className = '',
		children,
		...rest
	}: Props = $props();
</script>

<button
	{type}
	class="{buttonClasses(variant, size)} {fullWidth ? 'w-full' : ''} {className}"
	disabled={disabled || loading}
	{...rest}
>
	{#if loading}
		<LoadingSpinner size="sm" color="current" />
	{/if}
	{@render children()}
</button>
