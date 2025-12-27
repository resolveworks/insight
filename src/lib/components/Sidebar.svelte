<script lang="ts">
	import type { Snippet } from 'svelte';

	type Props = {
		title: string;
		children: Snippet;
	};

	let { title, children }: Props = $props();
</script>

<aside
	class="w-56 overflow-y-auto border-r border-neutral-700 bg-neutral-800 p-4"
>
	<h2 class="mb-3 text-xs font-medium uppercase tracking-wide text-neutral-400">
		{title}
	</h2>
	{@render children()}
</aside>
