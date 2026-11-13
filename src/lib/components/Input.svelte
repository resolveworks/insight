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
		oninput?: (value: string) => void;
	}

	let {
		id,
		label,
		hint,
		type = 'text',
		placeholder,
		disabled = false,
		value = $bindable(''),
		oninput,
	}: Props = $props();

	function handleInput(event: Event & { currentTarget: HTMLInputElement }) {
		value = event.currentTarget.value;
		oninput?.(value);
	}
</script>

<Field {id} {label} {hint}>
	<input
		{id}
		{type}
		{placeholder}
		{disabled}
		bind:value
		oninput={handleInput}
		class="w-full field"
	/>
</Field>
