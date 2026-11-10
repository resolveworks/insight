<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { HTMLButtonAttributes } from 'svelte/elements';

	type Variant = 'ghost' | 'danger';

	interface Props extends HTMLButtonAttributes {
		label: string;
		variant?: Variant;
		children: Snippet;
	}

	let {
		label,
		variant = 'ghost',
		type = 'button',
		children,
		...rest
	}: Props = $props();

	const variantClasses: Record<Variant, string> = {
		ghost: 'text-muted hover:bg-surface-dim hover:text-neutral-800',
		danger: 'text-muted hover:bg-error/10 hover:text-error',
	};
</script>

<button
	{type}
	aria-label={label}
	title={label}
	class="inline-flex items-center justify-center rounded-md p-1.5 transition-colors focus-visible:focus-ring disabled:opacity-50 disabled:cursor-not-allowed disabled:pointer-events-none {variantClasses[
		variant
	]}"
	{...rest}
>
	{@render children()}
</button>
