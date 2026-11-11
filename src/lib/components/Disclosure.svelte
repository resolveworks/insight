<script lang="ts">
	import type { Snippet } from 'svelte';
	import Icon from './Icon.svelte';

	interface Props {
		open?: boolean;
		summary: Snippet;
		children: Snippet;
	}

	let { open = $bindable(false), summary, children }: Props = $props();
</script>

<details bind:open class="group">
	<summary
		class="flex cursor-pointer list-none items-center gap-2 py-2 text-sm [&::-webkit-details-marker]:hidden"
	>
		<Icon
			name="chevron-right"
			size="sm"
			class="shrink-0 transition-transform group-open:rotate-90"
		/>
		{@render summary()}
	</summary>
	<div class="ps-6 pt-1 pb-2">{@render children()}</div>
</details>
