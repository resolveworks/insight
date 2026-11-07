<script lang="ts">
	import { resolve } from '$app/paths';
	import { listen } from '@tauri-apps/api/event';
	import { open } from '@tauri-apps/plugin-dialog';
	import { onMount } from 'svelte';
	import Button from '#lib/components/Button.svelte';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import {
		deleteDocument,
		getPipelineProgress,
		startImport,
	} from '#lib/api/document.js';
	import type {
		Document,
		PipelineProgress,
		StageProgress,
	} from '#lib/types/documents.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	let deletingDocument = $state<Document | null>(null);
	let pipelineProgress = $state<PipelineProgress>();

	const collectionId = $derived(data.collection.id);
	const documents = $derived(data.documents);
	const processing = $derived(pipelineProgress !== undefined);

	const stages = $derived(
		pipelineProgress
			? [
					{ name: 'Store', data: pipelineProgress.store },
					{ name: 'Extract', data: pipelineProgress.extract },
					{ name: 'Index', data: pipelineProgress.index },
				]
			: [],
	);

	function stageIsActive(stage: StageProgress): boolean {
		return stage.pending > 0 || stage.active > 0;
	}

	function pipelineIsActive(progress: PipelineProgress): boolean {
		return (
			stageIsActive(progress.store) ||
			stageIsActive(progress.extract) ||
			stageIsActive(progress.index)
		);
	}

	onMount(() => {
		void getPipelineProgress().then((active) => {
			pipelineProgress = active.find(
				(progress) => progress.collection_id === collectionId,
			);
		});
	});

	$effect(() => {
		const id = collectionId;
		let unlisten: (() => void) | undefined;
		let disposed = false;

		void listen<PipelineProgress>('pipeline-progress', (event) => {
			if (event.payload.collection_id !== id) return;
			pipelineProgress = pipelineIsActive(event.payload)
				? event.payload
				: undefined;
		}).then((stop) => {
			if (disposed) stop();
			else unlisten = stop;
		});

		return () => {
			disposed = true;
			unlisten?.();
		};
	});

	async function importPdf() {
		const files = await open({
			multiple: true,
			filters: [{ name: 'PDF', extensions: ['pdf'] }],
		});

		if (!files) return;

		const paths = Array.isArray(files) ? files : [files];
		try {
			await startImport(collectionId, paths);
		} catch (e) {
			console.error('Failed to start import:', e);
		}
	}

	function confirmDeleteDocument() {
		if (!deletingDocument) return;
		const documentId = deletingDocument.id;
		deletingDocument = null;
		deleteDocument(collectionId, documentId).catch((e) => {
			console.error('Failed to delete document:', e);
		});
	}
</script>

<div class="flex h-full flex-col">
	<!-- Content -->
	<div class="flex-1 overflow-y-auto p-6">
		{#if processing && stages.length > 0}
			<div class="mb-6 grid grid-cols-3 gap-4">
				{#each stages as stage (stage.name)}
					{@const total =
						stage.data.pending +
						stage.data.active +
						stage.data.completed +
						stage.data.failed}
					{@const done = stage.data.completed + stage.data.failed}
					{@const percent = total > 0 ? (done / total) * 100 : 0}
					<div class="flex flex-col gap-1">
						<div class="flex justify-between text-xs text-neutral-500">
							<span>{stage.name}</span>
							<span>{done}/{total}</span>
						</div>
						<div class="h-1.5 w-full rounded-full bg-neutral-200">
							<div
								class="h-1.5 rounded-full bg-primary-500 transition-all"
								style="width: {percent}%"
							></div>
						</div>
					</div>
				{/each}
			</div>
		{/if}

		{#if documents.length === 0 && processing}
			<div class="flex flex-col items-center justify-center py-12 text-center">
				<p class="text-neutral-500">Processing documents…</p>
				<p class="mt-1 max-w-sm text-sm text-neutral-400">
					Documents will appear here as they finish importing.
				</p>
			</div>
		{:else if documents.length === 0}
			<div class="flex flex-col items-center justify-center py-12 text-center">
				<p class="text-neutral-500">No documents yet</p>
				<p class="mt-1 max-w-sm text-sm text-neutral-400">
					The research agent answers from documents in this collection. Import a
					PDF to give it sources.
				</p>
				<Button class="mt-4" onclick={importPdf}>Import PDF</Button>
			</div>
		{:else}
			<div class="mb-4 flex items-center justify-end">
				<Button onclick={importPdf} disabled={processing}>Import PDF</Button>
			</div>

			<ul class="space-y-2">
				{#each documents as doc (doc.id)}
					<li
						class="flex items-center justify-between gap-4 rounded-lg border border-neutral-200 bg-surface-bright px-4 py-3 transition-colors hover:border-primary-300 hover:shadow-soft"
					>
						<a
							href={resolve(`collections/${collectionId}/documents/${doc.id}`)}
							class="min-w-0 flex-1"
						>
							<span
								class="text-neutral-800 transition-colors hover:text-primary-600"
								>{doc.name}</span
							>
							{#if doc.page_count !== null}
								<span class="ml-2 text-xs text-neutral-500"
									>{doc.page_count} pages</span
								>
							{/if}
						</a>
						<Button
							variant="danger"
							size="sm"
							onclick={() => (deletingDocument = doc)}
						>
							Delete
						</Button>
					</li>
				{/each}
			</ul>
		{/if}
	</div>
</div>

{#if deletingDocument}
	<ConfirmDialog
		title="Delete document"
		message="Delete &quot;{deletingDocument.name}&quot; from this collection? This removes the document and its extracted text from this collection."
		onConfirm={confirmDeleteDocument}
		onCancel={() => (deletingDocument = null)}
	/>
{/if}
