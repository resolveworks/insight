<script lang="ts">
	import LoadingSpinner from './LoadingSpinner.svelte';

	type Size = 'md' | 'lg';

	interface Props {
		label: string;
		size?: Size;
	}

	let { label, size = 'lg' }: Props = $props();
</script>

<div class="flex h-full items-center justify-center">
	<LoadingSpinner {size} ariaLabel={label} />
</div>
