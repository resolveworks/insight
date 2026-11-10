<script lang="ts">
	import type { HTMLSelectAttributes } from 'svelte/elements';
	import Field from './Field.svelte';

	interface Props extends Omit<HTMLSelectAttributes, 'value'> {
		options: { value: string; label: string }[];
		value?: string;
		label?: string;
		hint?: string;
	}

	let {
		options,
		value = $bindable(''),
		label,
		hint,
		id,
		class: className = '',
		...rest
	}: Props = $props();
</script>

<Field {id} {label} {hint}>
	<select
		{id}
		bind:value
		class="field w-full cursor-pointer {className}"
		{...rest}
	>
		{#each options as option (option.value)}
			<option value={option.value}>{option.label}</option>
		{/each}
	</select>
</Field>
