<script lang="ts">
	import type { Snippet } from 'svelte';

	type Width = 'sm' | 'md' | 'lg' | 'xl';

	interface Props {
		width?: Width;
		children: Snippet;
	}

	let { width = 'md', children }: Props = $props();

	const widthClasses: Record<Width, string> = {
		sm: 'max-w-sm',
		md: 'max-w-md',
		lg: 'max-w-lg',
		xl: 'max-w-xl',
	};
</script>

<div class="flex h-screen items-center justify-center bg-neutral-700">
	<div
		class="w-full {widthClasses[width]} rounded-lg bg-surface p-8 shadow-medium"
	>
		{@render children()}
	</div>
</div>
