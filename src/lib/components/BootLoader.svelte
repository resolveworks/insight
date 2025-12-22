<script lang="ts">
	type Props = {
		modelName?: string;
		phase: 'storage' | 'embedder' | 'ready';
	};

	let { modelName = 'embedding model', phase }: Props = $props();
</script>

<div class="flex h-screen flex-col items-center justify-center bg-slate-900">
	<div class="text-center">
		<div class="mb-6">
			<svg
				class="mx-auto h-12 w-12 animate-spin text-rose-500"
				fill="none"
				viewBox="0 0 24 24"
			>
				<circle
					class="opacity-25"
					cx="12"
					cy="12"
					r="10"
					stroke="currentColor"
					stroke-width="4"
				></circle>
				<path
					class="opacity-75"
					fill="currentColor"
					d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
				></path>
			</svg>
		</div>

		<h1 class="mb-2 text-xl font-semibold text-slate-100">
			{#if phase === 'storage'}
				Initializing...
			{:else if phase === 'embedder'}
				Loading {modelName}
			{:else}
				Starting Insight
			{/if}
		</h1>

		<p class="text-sm text-slate-400">
			{#if phase === 'embedder'}
				This may take 20-30 seconds
			{:else}
				Please wait...
			{/if}
		</p>
	</div>
</div>
