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
		// Clearing the field is what stops a second Enter or click from
		// creating the same collection while the first create is in flight.
		name = '';
		onCreate(collectionName);
	}
</script>

<form class="flex items-center gap-2" onsubmit={submit}>
	<div class="w-48">
		<Input placeholder="New collection..." bind:value={name} />
	</div>
	<Button type="submit" variant="solid" disabled={!name.trim()}>Create</Button>
</form>
