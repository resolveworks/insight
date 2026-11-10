<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		sidebar: Snippet;
		children: Snippet;
	}

	let { sidebar, children }: Props = $props();
</script>

<div class="flex h-full overflow-hidden">
	<aside
		class="flex w-64 shrink-0 flex-col overflow-hidden border-r border-line bg-surface-dim"
	>
		{@render sidebar()}
	</aside>
	<div class="min-w-0 flex-1 overflow-hidden bg-surface">
		{@render children()}
	</div>
</div>
