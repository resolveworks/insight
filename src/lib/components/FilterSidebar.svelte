<script lang="ts">
	import type { Snippet } from 'svelte';

	type Props = {
		children: Snippet;
	};

	let { children }: Props = $props();
</script>

<aside
	class="flex w-64 flex-col border-l border-neutral-900 bg-neutral-800 text-neutral-100"
>
	<div class="border-b border-neutral-900 p-4">
		<h2 class="text-xs font-medium uppercase tracking-wide text-neutral-400">
			Filters
		</h2>
	</div>
	<div class="flex-1 overflow-y-auto">
		{@render children()}
	</div>
</aside>
