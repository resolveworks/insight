<script lang="ts">
	type IconName =
		| 'arrow-left'
		| 'chevron-right'
		| 'plus'
		| 'trash'
		| 'close'
		| 'warning'
		| 'check';

	type Size = 'xs' | 'sm';

	type Shape =
		| { tag: 'path'; d: string }
		| { tag: 'circle'; cx: number; cy: number; r: number };

	interface IconDef {
		viewBox: string;
		filled?: boolean;
		shapes: Shape[];
	}

	interface Props {
		name: IconName;
		size?: Size;
		class?: string;
		label?: string;
	}

	let { name, size = 'sm', class: className = '', label }: Props = $props();

	const icons: Record<IconName, IconDef> = {
		'arrow-left': {
			viewBox: '0 0 24 24',
			shapes: [{ tag: 'path', d: 'M19 12H5M12 19l-7-7 7-7' }],
		},
		'chevron-right': {
			viewBox: '0 0 24 24',
			shapes: [{ tag: 'path', d: 'M9 18l6-6-6-6' }],
		},
		plus: {
			viewBox: '0 0 24 24',
			shapes: [{ tag: 'path', d: 'M12 4v16m8-8H4' }],
		},
		trash: {
			viewBox: '0 0 24 24',
			shapes: [
				{
					tag: 'path',
					d: 'M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16',
				},
			],
		},
		close: {
			viewBox: '0 0 24 24',
			shapes: [{ tag: 'path', d: 'M6 18L18 6M6 6l12 12' }],
		},
		warning: {
			viewBox: '0 0 20 20',
			filled: true,
			shapes: [
				{
					tag: 'path',
					d: 'M8.485 2.495c.673-1.167 2.357-1.167 3.03 0l6.28 10.875c.673 1.167-.17 2.625-1.516 2.625H3.72c-1.347 0-2.189-1.458-1.515-2.625L8.485 2.495ZM10 6a.75.75 0 0 1 .75.75v3.5a.75.75 0 0 1-1.5 0v-3.5A.75.75 0 0 1 10 6Zm0 9a1 1 0 1 0 0-2 1 1 0 0 0 0 2Z',
				},
			],
		},
		check: {
			viewBox: '0 0 20 20',
			filled: true,
			shapes: [
				{
					tag: 'path',
					d: 'M16.704 4.153a.75.75 0 0 1 .143 1.052l-8 10.5a.75.75 0 0 1-1.127.075l-4.5-4.5a.75.75 0 0 1 1.06-1.06l3.894 3.893 7.48-9.817a.75.75 0 0 1 1.05-.143Z',
				},
			],
		},
	};

	const sizeClasses: Record<Size, string> = {
		xs: 'h-3.5 w-3.5',
		sm: 'h-4 w-4',
	};

	const icon = $derived(icons[name]);
</script>

<svg
	class="{sizeClasses[size]} {className}"
	viewBox={icon.viewBox}
	fill={icon.filled ? 'currentColor' : 'none'}
	stroke={icon.filled ? undefined : 'currentColor'}
	stroke-width={icon.filled ? undefined : '2'}
	stroke-linecap={icon.filled ? undefined : 'round'}
	stroke-linejoin={icon.filled ? undefined : 'round'}
	fill-rule={icon.filled ? 'evenodd' : undefined}
	clip-rule={icon.filled ? 'evenodd' : undefined}
	role={label ? 'img' : undefined}
	aria-label={label}
	aria-hidden={label ? undefined : 'true'}
>
	{#each icon.shapes as shape, index (index)}
		{#if shape.tag === 'circle'}
			<circle cx={shape.cx} cy={shape.cy} r={shape.r} />
		{:else}
			<path d={shape.d} />
		{/if}
	{/each}
</svg>
