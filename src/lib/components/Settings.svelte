<script lang="ts">
	import ModelDownloadSelector from './ModelDownloadSelector.svelte';
</script>

<div class="flex h-full flex-col">
	<div class="flex-1 overflow-y-auto p-6">
		<div class="mx-auto max-w-2xl">
			<h1 class="mb-8 text-2xl font-semibold text-slate-100">Settings</h1>

			<section class="mb-8">
				<h2 class="mb-4 text-lg font-medium text-slate-200">AI Model</h2>
				<p class="mb-6 text-sm text-slate-400">
					Select and download models for local AI inference. Models are stored
					locally and run entirely on your machine.
				</p>
				<div class="rounded-lg border border-slate-700 bg-slate-800 p-6">
					<ModelDownloadSelector modelType="llm" showTitle={false} />
				</div>
			</section>

			<section class="mb-8">
				<h2 class="mb-4 text-lg font-medium text-slate-200">Semantic Search</h2>
				<p class="mb-6 text-sm text-slate-400">
					Enable semantic search to find documents by meaning, not just
					keywords. Requires downloading an embedding model.
				</p>
				<div class="rounded-lg border border-slate-700 bg-slate-800 p-6">
					<ModelDownloadSelector modelType="embedding" showTitle={false} />
				</div>
			</section>
		</div>
	</div>
</div>
