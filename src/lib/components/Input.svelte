<script lang="ts">
	import Field from './Field.svelte';

	interface Props {
		id?: string;
		label?: string;
		hint?: string;
		type?: 'text' | 'password';
		placeholder?: string;
		disabled?: boolean;
		value?: string;
	}

	let {
		id,
		label,
		hint,
		type = 'text',
		placeholder,
		disabled = false,
		value = $bindable(''),
	}: Props = $props();
</script>

<Field {id} {label} {hint}>
	<input {id} {type} {placeholder} {disabled} bind:value class="w-full field" />
</Field>
