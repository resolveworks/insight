<script lang="ts">
	import Alert from './Alert.svelte';
	import type { ImportOutcome } from '#lib/types/documents.js';

	interface Props {
		failures: ImportOutcome['failures'];
	}

	let { failures }: Props = $props();
</script>

<Alert variant="warning">
	<p class="font-medium">Some files could not be imported.</p>
	<ul class="mt-1 list-disc pl-5">
		{#each failures as failure, index (index)}
			<li>{failure.path}: {failure.message}</li>
		{/each}
	</ul>
</Alert>
