<script lang="ts">
	import type { Snippet } from 'svelte';

	type Tone = 'neutral' | 'error';

	interface Props {
		title: string;
		tone?: Tone;
		/** Inline status beside the title, e.g. an executing indicator. */
		summaryExtra?: Snippet;
		children: Snippet;
	}

	let { title, tone = 'neutral', summaryExtra, children }: Props = $props();

	const toneClasses: Record<Tone, { block: string; label: string }> = {
		neutral: {
			block: 'border-line bg-surface-bright',
			label: 'text-muted hover:text-neutral-800',
		},
		error: {
			block: 'border-error/50 bg-error/10',
			label: 'text-error',
		},
	};
</script>

<!-- Static `open`: the attribute is set once, so a manual fold sticks. -->
<details class="mx-4 rounded-md border {toneClasses[tone].block}" open>
	<summary
		class="flex cursor-pointer items-center gap-2 px-2 py-1 text-xs font-medium {toneClasses[
			tone
		].label}"
	>
		<span>{title}</span>
		{#if summaryExtra}{@render summaryExtra()}{/if}
	</summary>
	<div class="p-2 {tone === 'error' ? 'text-error' : 'text-neutral-700'}">
		{@render children()}
	</div>
</details>
