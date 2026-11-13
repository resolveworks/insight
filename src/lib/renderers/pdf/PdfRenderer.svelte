<script lang="ts" module>
	import { localEngine } from '@embedpdf/engine';
	import { renderPlugin } from '@embedpdf/svelte/render';
	import { searchPlugin } from '@embedpdf/svelte/search';
	import { stagePlugin } from '@embedpdf/svelte/stage';

	const engine = localEngine();
	const plugins = [stagePlugin(), renderPlugin(), searchPlugin()];
</script>

<script lang="ts">
	import { onMount } from 'svelte';
	import { DocumentGate, Viewer } from '@embedpdf/svelte/runtime';
	import { getDocumentSource } from '#lib/api/document.js';
	import { describeError } from '#lib/api/provider.js';
	import Alert from '#lib/components/Alert.svelte';
	import LoadingSpinner from '#lib/components/LoadingSpinner.svelte';
	import type { RendererProps } from '../registry.js';
	import PdfStage from './PdfStage.svelte';

	let {
		collectionId,
		documentId,
		page,
		query,
		activeHit,
		visible,
	}: RendererProps = $props();

	let source = $state<Uint8Array | null>(null);
	let error = $state<string | null>(null);

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

{#if error}
	<Alert>{error}</Alert>
{:else if source}
	<Viewer
		{engine}
		{plugins}
		initialDocuments={[
			{ source: { kind: 'bytes', id: documentId, bytes: source } },
		]}
	>
		{#snippet error(e)}
			<Alert>{describeError(e)}</Alert>
		{/snippet}
		<DocumentGate>
			{#snippet error(document)}
				<Alert>{document.error.message}</Alert>
			{/snippet}
			<PdfStage {page} {query} {activeHit} {visible} />
		</DocumentGate>
	</Viewer>
{:else}
	<div class="flex justify-center p-8">
		<LoadingSpinner size="lg" ariaLabel="Loading document" />
	</div>
{/if}
