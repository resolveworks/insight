<script lang="ts">
	interface Props {
		title: string;
		level?: 2 | 3;
		id?: string;
	}

	let { title, level = 2, id }: Props = $props();
</script>

<svelte:element
	this={`h${level}`}
	{id}
	class="text-sm font-semibold text-neutral-800"
>
	{title}
</svelte:element>
