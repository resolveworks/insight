<script lang="ts">
	import Button from './Button.svelte';
	import Input from './Input.svelte';

	interface Props {
		onCreate: (name: string) => void;
	}

	let { onCreate }: Props = $props();

	let name = $state('');

	function submit(event: SubmitEvent) {
		event.preventDefault();
		const collectionName = name.trim();
		if (!collectionName) return;
		onCreate(collectionName);
	}
</script>

<form class="flex items-center gap-2" onsubmit={submit}>
	<Input placeholder="New collection..." bind:value={name} class="w-48" />
	<Button type="submit">Create</Button>
</form>
