<script lang="ts">
	import LoadingSpinner from './LoadingSpinner.svelte';

	interface Props {
		label: string;
	}

	let { label }: Props = $props();
</script>

<div class="flex h-full items-center justify-center">
	<LoadingSpinner size="lg" ariaLabel={label} />
</div>
