<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		title?: string;
		onClose?: () => void;
		children: Snippet;
	}

	let { title, onClose, children }: Props = $props();

	const titleId = $props.id();
	const focusableSelector =
		'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"]), [contenteditable="true"]';

	let panel: HTMLDivElement | undefined = $state();

	function focusableElements() {
		if (!panel) return [];
		return Array.from(panel.querySelectorAll<HTMLElement>(focusableSelector));
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			e.preventDefault();
			onClose?.();
			return;
		}

		if (e.key !== 'Tab' || !panel) return;

		const focusable = focusableElements();
		if (focusable.length === 0) {
			e.preventDefault();
			panel.focus();
			return;
		}

		const first = focusable[0];
		const last = focusable[focusable.length - 1];
		const activeIndex = focusable.indexOf(
			document.activeElement as HTMLElement,
		);

		if (e.shiftKey && activeIndex <= 0) {
			e.preventDefault();
			last.focus();
		} else if (
			!e.shiftKey &&
			(activeIndex === -1 || activeIndex === focusable.length - 1)
		) {
			e.preventDefault();
			first.focus();
		}
	}

	function handleBackdropClick(e: MouseEvent) {
		if (e.target === panel?.parentElement) onClose?.();
	}

	function inertBackground(overlay: HTMLElement) {
		const changed: { element: HTMLElement; inert: boolean }[] = [];
		let node: HTMLElement | null = overlay;

		while (node && node !== document.body) {
			const parent: HTMLElement = node.parentElement!;
			for (const sibling of parent.children) {
				if (sibling === node || !(sibling instanceof HTMLElement)) continue;
				changed.push({ element: sibling, inert: sibling.inert });
				sibling.inert = true;
			}
			node = parent;
		}

		return () => {
			for (const { element, inert } of changed) element.inert = inert;
		};
	}

	$effect(() => {
		if (!panel) return;

		const previouslyFocused =
			document.activeElement instanceof HTMLElement
				? document.activeElement
				: null;
		const restoreBackground = inertBackground(panel.parentElement!);

		const target =
			panel.querySelector<HTMLElement>('[data-autofocus]') ??
			focusableElements()[0] ??
			panel;
		target.focus();

		return () => {
			restoreBackground();
			previouslyFocused?.focus();
		};
	});
</script>

<svelte:window onkeydown={handleKeydown} />

<div
	role="presentation"
	class="fixed inset-0 z-50 flex items-center justify-center bg-neutral-900/50 p-4"
	onclick={handleBackdropClick}
>
	<div
		bind:this={panel}
		role="dialog"
		aria-modal="true"
		aria-labelledby={title ? titleId : undefined}
		tabindex="-1"
		class="w-full max-w-md rounded-lg bg-surface-bright p-6 shadow-lg"
	>
		{#if title}
			<div class="mb-4 flex items-center justify-between">
				<h2 id={titleId} class="text-base font-semibold text-neutral-800">
					{title}
				</h2>
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
