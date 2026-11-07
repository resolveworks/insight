<script lang="ts">
	import ProviderSelector from './ProviderSelector.svelte';
</script>

<div
	class="flex h-full min-h-0 items-center justify-center overflow-y-auto p-6"
>
	<div class="panel w-full max-w-md p-8">
		<h1 class="mb-2 text-2xl text-neutral-800">Set up your language model</h1>
		<p class="mb-6 text-sm text-muted">
			Only the conversation is sent to the provider — documents never leave your
			machine.
		</p>
		<ProviderSelector />
	</div>
</div>
