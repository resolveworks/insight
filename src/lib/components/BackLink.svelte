<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		href: string;
		children: Snippet;
	}

	let { href, children }: Props = $props();
</script>

<!-- Callers pass already-resolved paths. -->
<!-- eslint-disable svelte/no-navigation-without-resolve -->
<a
	{href}
	class="inline-flex items-center gap-1 text-sm text-muted transition-colors hover:text-primary-600 focus-visible:focus-ring"
>
	<span aria-hidden="true">←</span>
	{@render children()}
</a>
<!-- eslint-enable svelte/no-navigation-without-resolve -->
