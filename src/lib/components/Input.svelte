<script lang="ts">
	import type { HTMLInputAttributes } from 'svelte/elements';

	interface Props extends Omit<HTMLInputAttributes, 'value'> {
		label?: string;
		value?: string;
		ghostText?: string;
		onAcceptGhost?: () => void;
	}

	let {
		label,
		id,
		value = $bindable(''),
		ghostText = '',
		onAcceptGhost,
		class: className = '',
		onkeydown,
		placeholder,
		...rest
	}: Props = $props();

	let effectivePlaceholder = $derived(ghostText || placeholder);

	const inputClasses =
		'flex-1 rounded-md border border-neutral-300 bg-surface-bright px-3 py-2 text-neutral-800 placeholder-neutral-400 focus:border-tertiary-400 focus:outline-none disabled:opacity-50';

	function handleKeyDown(
		e: KeyboardEvent & { currentTarget: HTMLInputElement },
	) {
		// Tab accepts ghost text only while the input is still empty.
		if (e.key === 'Tab' && ghostText && !value) {
			e.preventDefault();
			value = ghostText;
			onAcceptGhost?.();
			return;
		}

		onkeydown?.(e);
	}
</script>

{#if label}
	<div class="flex flex-col gap-2">
		<label for={id} class="block text-sm font-medium text-neutral-700">
			{label}
		</label>
		<input
			{id}
			bind:value
			onkeydown={handleKeyDown}
			placeholder={effectivePlaceholder}
			class="{inputClasses} {className}"
			{...rest}
		/>
	</div>
{:else}
	<input
		{id}
		bind:value
		onkeydown={handleKeyDown}
		placeholder={effectivePlaceholder}
		class="{inputClasses} {className}"
		{...rest}
	/>
{/if}
