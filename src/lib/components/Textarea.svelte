<script lang="ts">
	import Field from './Field.svelte';

	interface Props {
		id?: string;
		label?: string;
		hint?: string;
		rows?: number;
		placeholder?: string;
		disabled?: boolean;
		value?: string;
	}

	let {
		id,
		label,
		hint,
		rows,
		placeholder,
		disabled = false,
		value = $bindable(''),
	}: Props = $props();
</script>

<Field {id} {label} {hint}>
	<textarea
		{id}
		{rows}
		{placeholder}
		{disabled}
		bind:value
		class="w-full resize-none field"></textarea>
</Field>
