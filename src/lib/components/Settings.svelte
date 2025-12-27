<script lang="ts">
	import ModelDownloadSelector from './ModelDownloadSelector.svelte';
	import ProviderSelector from './ProviderSelector.svelte';
	import { embeddingModelConfig } from '$lib/models/config';
</script>

<div class="flex h-full flex-col">
	<div class="flex-1 overflow-y-auto p-6">
		<div class="mx-auto max-w-2xl">
			<h1 class="mb-8 text-2xl font-semibold text-slate-100">Settings</h1>

			<section class="mb-8">
				<h2 class="mb-4 text-lg font-medium text-slate-200">Language Model</h2>
				<p class="mb-6 text-sm text-slate-400">
					Choose a provider for chat. Run models locally on your machine, or
					connect to OpenAI or Anthropic APIs.
				</p>
				<div class="rounded-lg border border-slate-700 bg-slate-800 p-6">
					<ProviderSelector />
				</div>
			</section>

			<section class="mb-8">
				<h2 class="mb-4 text-lg font-medium text-slate-200">Embedding Model</h2>
				<p class="mb-6 text-sm text-slate-400">
					Enable semantic search to find documents by meaning, not just
					keywords. Requires downloading an embedding model.
				</p>
				<div class="rounded-lg border border-slate-700 bg-slate-800 p-6">
					<ModelDownloadSelector config={embeddingModelConfig} />
				</div>
			</section>
		</div>
	</div>
</div>
