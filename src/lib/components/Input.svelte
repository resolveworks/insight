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
		placeholder,
		...rest
	}: Props = $props();

	const inputClasses = 'field';
</script>

{#if label}
	<div class="flex flex-col gap-2">
		<label for={id} class="block text-sm font-medium text-neutral-700">
			{label}
		</label>
		<input
			{id}
			bind:value
			{placeholder}
			class="{inputClasses} {className}"
			{...rest}
		/>
	</div>
{:else}
	<input
		{id}
		bind:value
		{placeholder}
		class="{inputClasses} {className}"
		{...rest}
	/>
{/if}
