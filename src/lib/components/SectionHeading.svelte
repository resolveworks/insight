<script lang="ts">
	interface Props {
		title: string;
		id?: string;
	}

	let { title, id }: Props = $props();
</script>

<h2 {id} class="text-sm font-semibold text-neutral-800">
	{title}
</h2>
