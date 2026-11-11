<script lang="ts">
	import Alert from './Alert.svelte';
	import Card from './Card.svelte';
	import Disclosure from './Disclosure.svelte';
	import LoadingSpinner from './LoadingSpinner.svelte';
	import SectionHeading from './SectionHeading.svelte';

	interface Props {
		chunks: string[] | null;
		error: string | null;
		loading: boolean;
		open?: boolean;
	}

	let { chunks, error, loading, open = $bindable(false) }: Props = $props();
</script>

<Disclosure bind:open>
	{#snippet summary()}
		<SectionHeading title="Text Chunks" />
		{#if chunks}
			<span class="text-xs font-normal text-neutral-400"
				>({chunks.length} chunks)</span
			>
		{/if}
	{/snippet}

	{#if loading}
		<LoadingSpinner size="sm" ariaLabel="Loading chunks" />
	{:else if error}
		<Alert variant="warning">{error}</Alert>
	{:else if chunks && chunks.length > 0}
		<div class="flex flex-col gap-3">
			{#each chunks as chunk, i (i)}
				<Card padding="sm">
					<div
						class="mb-2 flex items-center justify-between text-xs text-neutral-400"
					>
						<span>Chunk {i + 1}</span>
						<span>{chunk.length} chars</span>
					</div>
					<p
						class="whitespace-pre-wrap break-words font-sans text-sm leading-relaxed text-neutral-700"
					>
						{chunk}
					</p>
				</Card>
			{/each}
		</div>
	{:else}
		<p class="text-sm italic text-neutral-400">No chunks generated</p>
	{/if}
</Disclosure>
