<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { HTMLAnchorAttributes } from 'svelte/elements';
	import {
		buttonClasses,
		type ButtonSize,
		type ButtonVariant,
	} from './button-classes';

	interface Props extends HTMLAnchorAttributes {
		href: string;
		variant: ButtonVariant;
		size?: ButtonSize;
		children: Snippet;
	}

	let { href, variant, size = 'md', children, ...rest }: Props = $props();
</script>

<!-- Callers pass already-resolved paths. -->
<!-- eslint-disable svelte/no-navigation-without-resolve -->
<a {href} class={buttonClasses(variant, size)} {...rest}>
	{@render children()}
</a>
<!-- eslint-enable svelte/no-navigation-without-resolve -->
