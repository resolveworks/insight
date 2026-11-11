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
		type = 'text',
		value = $bindable(''),
		class: className = '',
		...rest
	}: Props = $props();
</script>

{#snippet control()}
	<input {id} {type} bind:value class="field w-full {className}" {...rest} />
{/snippet}

<Field {id} {label} {hint}>
	{@render control()}
</Field>
