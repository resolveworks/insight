<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		children: Snippet;
	}

	let { children }: Props = $props();
</script>

<dl class="divide-y divide-neutral-200 border-t border-neutral-200">
	{@render children()}
</dl>
