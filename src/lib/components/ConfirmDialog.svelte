<script lang="ts">
	import Modal from './Modal.svelte';
	import Button from './Button.svelte';

	interface Props {
		title: string;
		message: string;
		confirmLabel?: string;
		onConfirm: () => void;
		onCancel: () => void;
	}

	let {
		title,
		message,
		confirmLabel = 'Delete',
		onConfirm,
		onCancel,
	}: Props = $props();
</script>

<Modal {title} onClose={onCancel}>
	<p class="text-sm text-neutral-500">{message}</p>
	{#snippet footer()}
		<Button variant="ghost" onclick={onCancel} data-autofocus>Cancel</Button>
		<Button variant="danger" onclick={onConfirm}>{confirmLabel}</Button>
	{/snippet}
</Modal>
