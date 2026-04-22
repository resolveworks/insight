<script lang="ts">
	import type { Snippet } from 'svelte';

	type Props = {
		children: Snippet;
	};

	let { children }: Props = $props();
</script>

<aside
	class="flex w-64 flex-col border-l border-primary-700 bg-primary-600 text-surface"
>
	<div class="border-b border-primary-700 p-4">
		<h2 class="text-xs font-medium uppercase tracking-wide text-primary-200">
			Filters
		</h2>
	</div>
	<div class="flex-1 overflow-y-auto">
		{@render children()}
	</div>
</aside>
