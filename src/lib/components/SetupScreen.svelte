<script lang="ts">
	import CenteredLayout from './CenteredLayout.svelte';
	import ProviderSelector from './ProviderSelector.svelte';
</script>

<CenteredLayout width="md">
	<h1 class="mb-2 text-2xl text-neutral-800">Set up your language model</h1>
	<p class="mb-6 text-sm text-neutral-500">
		Only the conversation is sent to the provider — documents never leave your
		machine.
	</p>
	<div class="rounded-lg border border-neutral-200 bg-surface-bright p-6">
		<ProviderSelector />
	</div>
</CenteredLayout>
