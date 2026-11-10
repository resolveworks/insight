<script lang="ts">
	interface Props {
		value: number;
		max: number;
		label?: string;
		detail?: string;
	}

	let { value, max, label, detail }: Props = $props();

	const percent = $derived(max === 0 ? 0 : (value / max) * 100);
</script>

{#if label || detail}
	<div
		class="mb-1.5 flex items-center justify-between gap-4 text-xs text-muted"
	>
		<span>{label}</span>
		<span>{detail}</span>
	</div>
{/if}
<div
	role="progressbar"
	aria-valuenow={value}
	aria-valuemin="0"
	aria-valuemax={max}
	aria-label={label}
	class="h-1.5 w-full rounded-full bg-surface-dim"
>
	<div
		class="h-1.5 rounded-full bg-primary-500 transition-all"
		style:width="{percent}%"
	></div>
</div>
