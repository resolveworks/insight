<script lang="ts">
	import ModelDownloadSelector from './ModelDownloadSelector.svelte';
	import CenteredLayout from './CenteredLayout.svelte';
	import { embeddingModelConfig } from '$lib/models/config';

	type Props = {
		onComplete: () => void;
	};

	let { onComplete }: Props = $props();

	function handleConfigured(modelId: string | null) {
		if (modelId) {
			onComplete();
		}
	}
</script>

<CenteredLayout width="xl">
	<div class="mb-8 text-center">
		<h1 class="mb-2 text-3xl font-bold text-neutral-100">Welcome to Insight</h1>
		<p class="text-neutral-400">
			Before you begin, please set up an embedding model for semantic search.
			This enables searching documents by meaning, not just keywords.
		</p>
	</div>

	<div class="rounded-lg border border-neutral-700 bg-neutral-800 p-6">
		<ModelDownloadSelector
			config={embeddingModelConfig}
			onConfigured={handleConfigured}
		/>
	</div>

	<p class="mt-6 text-center text-sm text-neutral-500">
		The embedding model will be downloaded and loaded. This may take a few
		minutes on first setup.
	</p>
</CenteredLayout>
