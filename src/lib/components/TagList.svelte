<script lang="ts">
	interface Props {
		tags: string[];
	}

	let { tags }: Props = $props();
</script>

{#if tags.length > 0}
	<div class="flex flex-wrap gap-1">
		{#each tags as tag (tag)}
			<span
				class="rounded-sm bg-neutral-100 px-2 py-0.5 text-xs text-neutral-700"
				>{tag}</span
			>
		{/each}
	</div>
{:else}
	<span class="italic text-neutral-400">(none)</span>
{/if}
