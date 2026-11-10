<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		open?: boolean;
		summary: Snippet;
		children: Snippet;
		class?: string;
		summaryClass?: string;
	}

	let {
		open = $bindable(false),
		summary,
		children,
		class: className = '',
		summaryClass = '',
	}: Props = $props();
</script>

<details bind:open class="group {className}">
	<summary
		class="cursor-pointer list-none [&::-webkit-details-marker]:hidden {summaryClass}"
	>
		{@render summary()}
	</summary>
	{@render children()}
</details>
