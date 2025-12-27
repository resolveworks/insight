<script lang="ts">
	type Size = 'sm' | 'md' | 'lg';
	type Color = 'slate' | 'emerald' | 'neutral' | 'current';

	interface Props {
		size?: Size;
		color?: Color;
		class?: string;
	}

	let { size = 'md', color = 'slate', class: className = '' }: Props = $props();

	const sizeClasses: Record<Size, string> = {
		sm: 'h-4 w-4',
		md: 'h-8 w-8',
		lg: 'h-12 w-12',
	};

	const colorClasses: Record<Color, string> = {
		slate: 'text-slate-400',
		emerald: 'text-emerald-500',
		neutral: 'text-neutral-500',
		current: 'text-current',
	};
</script>

<svg
	class="animate-spin {sizeClasses[size]} {colorClasses[color]} {className}"
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
