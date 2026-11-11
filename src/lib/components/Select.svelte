<script lang="ts">
	import Field from './Field.svelte';

	interface Props {
		options: { value: string; label: string }[];
		value?: string;
		label?: string;
		hint?: string;
		id?: string;
		disabled?: boolean;
		onchange?: (value: string) => void;
	}

	let {
		options,
		value = $bindable(''),
		label,
		hint,
		id,
		disabled = false,
		onchange,
	}: Props = $props();

	function handleChange(event: Event & { currentTarget: HTMLSelectElement }) {
		value = event.currentTarget.value;
		onchange?.(value);
	}
</script>

<Field {id} {label} {hint}>
	<select
		{id}
		{disabled}
		{value}
		onchange={handleChange}
		class="w-full cursor-pointer field"
	>
		{#each options as option (option.value)}
			<option value={option.value}>{option.label}</option>
		{/each}
	</select>
</Field>
