<script lang="ts">
	import type { HTMLTextareaAttributes } from 'svelte/elements';
	import Field from './Field.svelte';

	interface Props extends Omit<HTMLTextareaAttributes, 'value'> {
		label?: string;
		hint?: string;
		rows?: number;
		value?: string;
	}

	let {
		label,
		hint,
		id,
		rows,
		value = $bindable(''),
		class: className = '',
		...rest
	}: Props = $props();
</script>

{#snippet control()}
	<textarea
		{id}
		{rows}
		bind:value
		class="w-full resize-none field {className}"
		{...rest}></textarea>
{/snippet}

<Field {id} {label} {hint}>
	{@render control()}
</Field>
