<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		labelledBy?: string;
		children: Snippet;
	}

	let { labelledBy, children }: Props = $props();
</script>

<ul class="flex flex-col gap-2" aria-labelledby={labelledBy}>
	{@render children()}
</ul>
