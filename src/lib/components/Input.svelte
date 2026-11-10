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

{#snippet control()}
	<input {id} bind:value class="field {className}" {...rest} />
{/snippet}

{#if label || hint}
	<Field {id} {label} {hint}>
		{@render control()}
	</Field>
{:else}
	{@render control()}
{/if}
