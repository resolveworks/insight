<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { HTMLButtonAttributes } from 'svelte/elements';
	import LoadingSpinner from './LoadingSpinner.svelte';

	type Variant = 'primary' | 'secondary' | 'ghost' | 'danger';
	type Size = 'sm' | 'md';

	interface Props extends HTMLButtonAttributes {
		variant?: Variant;
		size?: Size;
		loading?: boolean;
		fullWidth?: boolean;
		children: Snippet;
	}

	let {
		variant = 'primary',
		size = 'md',
		loading = false,
		fullWidth = false,
		disabled,
		class: className = '',
		children,
		...rest
	}: Props = $props();

	const baseClasses =
		'rounded-md font-medium transition-colors cursor-pointer focus-visible:focus-ring disabled:opacity-50 disabled:cursor-not-allowed';

	let variantClasses = $derived.by(() => ({
		primary: 'bg-primary-600 hover:bg-primary-700 text-white',
		secondary:
			'bg-secondary-400 hover:bg-secondary-500 text-neutral-800 border border-secondary-600',
		ghost: 'bg-transparent hover:bg-neutral-200 text-neutral-700',
		danger: 'bg-error hover:bg-error-hover text-white',
	}));

	const sizeClasses: Record<Size, string> = {
		sm: 'px-3 py-1.5 text-sm',
		md: 'px-4 py-2',
	};
</script>

<button
	class="{baseClasses} {variantClasses[variant]} {sizeClasses[size]} {fullWidth
		? 'w-full'
		: ''} {className}"
	disabled={disabled || loading}
	{...rest}
>
	{#if loading}
		<LoadingSpinner size="sm" color="current" class="mr-2 inline-block" />
	{/if}
	{@render children()}
</button>
