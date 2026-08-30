<script lang="ts">
	import ProviderSelector from './ProviderSelector.svelte';
</script>

<div class="flex h-full flex-col bg-surface">
	<div class="flex-1 overflow-y-auto p-6">
		<div class="mx-auto max-w-2xl">
			<h1 class="mb-8 text-2xl text-neutral-800">Settings</h1>

			<section class="mb-8">
				<h2 class="mb-4 text-lg font-medium text-neutral-700">
					Language Model
				</h2>
				<p class="mb-6 text-sm text-neutral-500">
					Choose a remote provider for chat. Connect to the OpenAI or Anthropic
					API.
				</p>
				<div class="rounded-lg border border-neutral-200 bg-surface-bright p-6">
					<ProviderSelector />
				</div>
			</section>
		</div>
	</div>
</div>
