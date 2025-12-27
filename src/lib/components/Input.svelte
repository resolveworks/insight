<script lang="ts">
	import type { HTMLInputAttributes } from 'svelte/elements';

	interface Props extends Omit<HTMLInputAttributes, 'value'> {
		label?: string;
		value?: string;
	}

	let {
		label,
		id,
		value = $bindable(''),
		class: className = '',
		...rest
	}: Props = $props();

	const inputClasses =
		'flex-1 rounded-md border border-neutral-600 bg-neutral-900 px-3 py-2 text-neutral-100 placeholder-neutral-500 focus:border-amber-500 focus:outline-none disabled:opacity-50';
</script>

{#if label}
	<div class="flex flex-col gap-2">
		<label for={id} class="block text-sm font-medium text-neutral-300">
			{label}
		</label>
		<input {id} bind:value class="{inputClasses} {className}" {...rest} />
	</div>
{:else}
	<input {id} bind:value class="{inputClasses} {className}" {...rest} />
{/if}
