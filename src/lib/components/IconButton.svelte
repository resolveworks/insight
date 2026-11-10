<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { HTMLButtonAttributes } from 'svelte/elements';

	type Variant = 'ghost' | 'danger';
	type Size = 'sm' | 'md';

	interface Props extends HTMLButtonAttributes {
		label: string;
		variant?: Variant;
		size?: Size;
		children: Snippet;
	}

	let {
		label,
		variant = 'ghost',
		size = 'md',
		type = 'button',
		class: className = '',
		children,
		...rest
	}: Props = $props();

	const variantClasses: Record<Variant, string> = {
		ghost: 'hover:bg-surface-dim hover:text-neutral-700',
		danger: 'hover:text-error',
	};

	const sizeClasses: Record<Size, string> = {
		sm: 'p-1.5',
		md: 'p-2',
	};
</script>

<button
	{type}
	aria-label={label}
	title={label}
	class="inline-flex items-center justify-center rounded-md text-subtle transition-colors focus-visible:focus-ring disabled:opacity-50 disabled:cursor-not-allowed {variantClasses[
		variant
	]} {sizeClasses[size]} {className}"
	{...rest}
>
	{@render children()}
</button>
