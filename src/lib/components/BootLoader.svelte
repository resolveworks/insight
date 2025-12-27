<script lang="ts">
	import CenteredLayout from './CenteredLayout.svelte';
	import LoadingSpinner from './LoadingSpinner.svelte';
</script>

<CenteredLayout width="sm">
	<div class="text-center">
		<div class="mb-6 flex justify-center">
			<LoadingSpinner size="lg" color="slate" />
		</div>

		<h1 class="mb-2 text-xl font-semibold text-neutral-100">
			Starting Insight
		</h1>
		<p class="text-sm text-neutral-400">Please wait...</p>
	</div>
</CenteredLayout>
