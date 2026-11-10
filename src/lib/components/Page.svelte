<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		header: Snippet;
		children: Snippet;
	}

	let { header, children }: Props = $props();
</script>

<div class="flex h-full flex-col bg-surface">
	{@render header()}
	<div class="min-h-0 flex-1">
		{@render children()}
	</div>
</div>
