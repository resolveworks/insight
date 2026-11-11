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

<div class="flex flex-col gap-1.5">
	{#if label || detail}
		<div
			class="flex items-center justify-between gap-4 text-xs text-neutral-500"
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
		class="h-1.5 w-full rounded-full bg-neutral-100"
	>
		<div
			class="h-1.5 rounded-full bg-neutral-800 transition-all"
			style:width="{percent}%"
		></div>
	</div>
</div>
