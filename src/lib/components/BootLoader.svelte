<script lang="ts">
	import CenteredLayout from './CenteredLayout.svelte';
	import LoadingSpinner from './LoadingSpinner.svelte';
</script>

<CenteredLayout width="sm">
	<div class="text-center">
		<div class="mb-6 flex justify-center">
			<LoadingSpinner size="lg" color="primary" />
		</div>

		<h1 class="mb-2 text-xl text-neutral-800">Starting Insight</h1>
		<p class="text-sm text-neutral-500">Please wait...</p>
	</div>
</CenteredLayout>
