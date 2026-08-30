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
	<p class="text-sm text-neutral-600">{message}</p>
	<div class="mt-6 flex justify-end gap-2">
		<Button variant="secondary" onclick={onCancel} data-autofocus>Cancel</Button
		>
		<Button variant="danger" onclick={onConfirm}>{confirmLabel}</Button>
	</div>
</Modal>
