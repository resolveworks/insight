<script lang="ts">
	import type { Snippet } from 'svelte';

	type Props = {
		title: string;
		children: Snippet;
	};

	let { title, children }: Props = $props();
</script>

<aside
	class="w-64 overflow-y-auto border-r border-primary-700 bg-primary-600 p-4 text-surface"
>
	<h2 class="mb-3 text-xs font-medium uppercase tracking-wide text-primary-200">
		{title}
	</h2>
	{@render children()}
</aside>
