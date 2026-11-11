<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		label: string;
		mono?: boolean;
		children: Snippet;
	}

	let { label, mono = false, children }: Props = $props();
</script>

<div class="flex items-baseline justify-between gap-4 py-2">
	<dt class="text-sm text-neutral-500">{label}</dt>
	<dd class="text-end text-sm text-neutral-800 {mono ? 'font-mono' : ''}">
		{@render children()}
	</dd>
</div>
