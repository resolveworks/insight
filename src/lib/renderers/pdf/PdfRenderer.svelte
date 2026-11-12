<script lang="ts">
	import { onMount } from 'svelte';
	import { PDFViewer, type Viewer } from '@embedpdf/viewer-svelte';
	import { getDocumentSource } from '#lib/api/document.js';
	import { describeError } from '#lib/api/provider.js';
	import Alert from '#lib/components/Alert.svelte';
	import LoadingSpinner from '#lib/components/LoadingSpinner.svelte';
	import type { RendererProps } from '../registry.js';

	let { collectionId, documentId, page }: RendererProps = $props();

	let source = $state<Uint8Array | null>(null);
	let error = $state<string | null>(null);
	let viewer = $state.raw<Viewer | null>(null);
	let documentOpened = $state(false);

	function onready(ready: Viewer) {
		viewer = ready;
		ready.documents.onOpened(() => {
			documentOpened = true;
		});
	}

	$effect(() => {
		if (!viewer || !documentOpened || page === null) return;
		viewer.get('stage').goToPage(page - 1);
	});

	onMount(() => {
		let active = true;
		getDocumentSource(collectionId, documentId).then(
			(bytes) => {
				if (active) source = bytes;
			},
			(e) => {
				if (active) error = describeError(e);
			},
		);
		return () => {
			active = false;
		};
	});
</script>

<div class="h-full">
	{#if error}
		<Alert>{error}</Alert>
	{:else if source}
		<PDFViewer src={source} {onready} class="block h-full" />
	{:else}
		<div class="flex justify-center p-8">
			<LoadingSpinner size="lg" ariaLabel="Loading document" />
		</div>
	{/if}
</div>
