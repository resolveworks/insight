<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		title?: string;
		onClose?: () => void;
		children: Snippet;
	}

	let { title, onClose, children }: Props = $props();

	let panel: HTMLDivElement | undefined = $state();

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			e.preventDefault();
			onClose?.();
		}
	}

	function handleBackdropClick(e: MouseEvent) {
		if (e.target === panel?.parentElement) onClose?.();
	}

	// Autofocus the first focusable element inside the panel
	$effect(() => {
		if (!panel) return;
		const focusable = panel.querySelector<HTMLElement>('[data-autofocus]');
		focusable?.focus();
	});
</script>

<svelte:window onkeydown={handleKeydown} />

<div
	role="presentation"
	class="fixed inset-0 z-50 flex items-center justify-center bg-neutral-900/50 p-4"
	onclick={handleBackdropClick}
	onkeydown={(e) => e.key === 'Escape' && onClose?.()}
>
	<div
		bind:this={panel}
		role="dialog"
		aria-modal="true"
		aria-label={title}
		class="w-full max-w-md rounded-lg bg-surface-bright p-6 shadow-lg"
	>
		{#if title}
			<div class="mb-4 flex items-center justify-between">
				<h2 class="text-base font-semibold text-neutral-800">{title}</h2>
				<button
					onclick={onClose}
					class="cursor-pointer rounded p-1 text-neutral-400 transition-colors hover:text-neutral-700"
					aria-label="Close dialog"
					title="Close"
				>
					<svg
						class="h-4 w-4"
						viewBox="0 0 24 24"
						fill="none"
						stroke="currentColor"
					>
						<path
							stroke-linecap="round"
							stroke-linejoin="round"
							stroke-width="2"
							d="M6 18L18 6M6 6l12 12"
						/>
					</svg>
				</button>
			</div>
		{/if}
		{@render children()}
	</div>
</div>
