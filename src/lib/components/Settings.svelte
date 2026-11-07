<script lang="ts">
	import PageHeader from './PageHeader.svelte';
	import ProviderSelector from './ProviderSelector.svelte';
</script>

<div class="flex h-full flex-col bg-surface">
	<PageHeader title="Settings" />
	<div class="flex-1 overflow-y-auto p-6">
		<div class="mx-auto max-w-2xl">
			<section class="mb-8">
				<h2 class="mb-4 text-lg font-medium text-neutral-700">
					Language Model
				</h2>
				<p class="mb-6 text-sm text-muted">
					Choose a chat provider from the registry. Only the conversation is
					sent to the provider — documents never leave your machine.
				</p>
				<div class="panel p-6">
					<ProviderSelector />
				</div>
			</section>
		</div>
	</div>
</div>
