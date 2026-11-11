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
		class="flex w-64 shrink-0 flex-col overflow-hidden border-e border-neutral-200 bg-neutral-100"
	>
		{@render sidebar()}
	</aside>
	<div class="min-w-0 flex-1 overflow-hidden bg-neutral-50">
		{@render children()}
	</div>
</div>
