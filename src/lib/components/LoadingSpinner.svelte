<script lang="ts">
	type Size = 'xs' | 'sm' | 'lg';
	type Color = 'primary' | 'current';

	interface Props {
		size?: Size;
		color?: Color;
		ariaLabel?: string;
	}

	let {
		size = 'sm',
		color = 'primary',
		ariaLabel = 'Loading',
	}: Props = $props();

	const sizeClasses: Record<Size, string> = {
		xs: 'h-3.5 w-3.5',
		sm: 'h-4 w-4',
		lg: 'h-12 w-12',
	};

	const colorClasses: Record<Color, string> = {
		primary: 'text-blue-500',
		current: 'text-current',
	};
</script>

<svg
	role="status"
	aria-label={ariaLabel}
	class="animate-spin {sizeClasses[size]} {colorClasses[color]}"
	fill="none"
	viewBox="0 0 24 24"
>
	<circle
		class="opacity-25"
		cx="12"
		cy="12"
		r="10"
		stroke="currentColor"
		stroke-width="4"
	></circle>
	<path
		class="opacity-75"
		fill="currentColor"
		d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
	></path>
</svg>
