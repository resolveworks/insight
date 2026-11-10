<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		labelledBy?: string;
		children: Snippet;
	}

	let { labelledBy, children }: Props = $props();
</script>

<ul class="space-y-2" aria-labelledby={labelledBy}>
	{@render children()}
</ul>
