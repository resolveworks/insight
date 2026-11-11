<script lang="ts">
	import type { Snippet } from 'svelte';
	import {
		buttonClasses,
		type ButtonSize,
		type ButtonVariant,
	} from './button-classes';

	interface Props {
		href: string;
		variant: ButtonVariant;
		size?: ButtonSize;
		children: Snippet;
	}

	let { href, variant, size = 'md', children }: Props = $props();
</script>

<!-- Callers pass already-resolved paths. -->
<!-- eslint-disable svelte/no-navigation-without-resolve -->
<a {href} class={buttonClasses(variant, size)}>
	{@render children()}
</a>
<!-- eslint-enable svelte/no-navigation-without-resolve -->
