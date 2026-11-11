<script lang="ts">
	import type { Snippet } from 'svelte';
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
	{disabled}
	aria-label={label}
	title={label}
	{onclick}
>
	{@render children()}
</button>
