<script lang="ts">
	import type { HTMLInputAttributes } from 'svelte/elements';
	import Field from './Field.svelte';

	interface Props extends Omit<HTMLInputAttributes, 'value'> {
		label?: string;
		hint?: string;
		value?: string;
	}

	let {
		label,
		hint,
		id,
		value = $bindable(''),
		class: className = '',
		...rest
	}: Props = $props();
</script>

<Field {id} {label} {hint}>
	<input {id} bind:value class="field {className}" {...rest} />
</Field>
