<script lang="ts">
	import type { Snippet } from 'svelte';
	import Icon from './Icon.svelte';
	import Button from './Button.svelte';

	interface Props {
		title?: string;
		onClose?: () => void;
		children: Snippet;
		footer?: Snippet;
	}

	let { title, onClose, children, footer }: Props = $props();

	const titleId = $props.id();

	let dialog: HTMLDialogElement | undefined = $state();

	function handleCancel(event: Event) {
		event.preventDefault();
		onClose?.();
	}

	function handleBackdropClick(event: MouseEvent) {
		if (event.target !== dialog) return;

		const rect = dialog.getBoundingClientRect();
		const inside =
			event.clientX >= rect.left &&
			event.clientX <= rect.right &&
			event.clientY >= rect.top &&
			event.clientY <= rect.bottom;

		if (!inside) onClose?.();
	}

	$effect(() => {
		if (!dialog) return;

		dialog.showModal();

		return () => {
			if (dialog?.open) dialog.close();
		};
	});
</script>

<dialog
	bind:this={dialog}
	aria-labelledby={title ? titleId : undefined}
	oncancel={handleCancel}
	onclick={handleBackdropClick}
	class="m-auto w-full max-w-md rounded-lg bg-white p-6 shadow-xl backdrop:bg-black/40"
>
	{#if title}
		<div class="mb-4 flex items-center justify-between">
			<h2 id={titleId} class="text-base font-semibold text-neutral-800">
				{title}
			</h2>
			<Button
				size="icon"
				variant="ghost"
				label="Close dialog"
				onclick={onClose}
			>
				<Icon name="close" size="sm" />
			</Button>
		</div>
	{/if}
	<div class="flex flex-col gap-3">
		{@render children()}
	</div>
	{#if footer}
		<div class="mt-6 flex justify-end gap-2">
			{@render footer()}
		</div>
	{/if}
</dialog>
