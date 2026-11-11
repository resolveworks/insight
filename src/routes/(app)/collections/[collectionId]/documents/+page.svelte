<script lang="ts">
	import { resolve } from '$app/paths';
	import { listen } from '@tauri-apps/api/event';
	import { open } from '@tauri-apps/plugin-dialog';
	import { onMount } from 'svelte';
	import Alert from '#lib/components/Alert.svelte';
	import Button from '#lib/components/Button.svelte';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import CollectionHeader from '#lib/components/CollectionHeader.svelte';
	import DeletedDocumentRow from '#lib/components/DeletedDocumentRow.svelte';
	import DocumentRow from '#lib/components/DocumentRow.svelte';
	import EmptyState from '#lib/components/EmptyState.svelte';
	import ImportFailures from '#lib/components/ImportFailures.svelte';
	import List from '#lib/components/List.svelte';
	import Page from '#lib/components/Page.svelte';
	import PageSection from '#lib/components/PageSection.svelte';
	import PipelineProgress from '#lib/components/PipelineProgress.svelte';
	import {
		deleteDocument,
		getPipelineProgress,
		startImport,
		undeleteDocument,
	} from '#lib/api/document.js';
	import { describeError } from '#lib/api/provider.js';
	import { pipelineIsActive } from '#lib/pipeline.js';
	import type {
		Document,
		ImportOutcome,
		PipelineProgress as PipelineProgressState,
	} from '#lib/types/documents.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	let deletingDocument = $state<Document | null>(null);
	let pipelineProgress = $state<PipelineProgressState>();
	let importFailures = $state<ImportOutcome['failures']>([]);
	let importError = $state<string | null>(null);

	const collectionId = $derived(data.collection.id);
	const documents = $derived(data.documents);
	const deletedDocuments = $derived(data.deletedDocuments);
	const processing = $derived(pipelineProgress !== undefined);

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

		void listen<PipelineProgressState>('pipeline-progress', (event) => {
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
		importFailures = [];
		importError = null;

		const files = await open({
			multiple: true,
			filters: [
				{
					name: 'Documents',
					extensions: [
						'pdf',
						'docx',
						'xlsx',
						'pptx',
						'epub',
						'html',
						'htm',
						'xhtml',
						'txt',
						'md',
						'rst',
					],
				},
			],
		});

		if (!files) return;

		const paths = Array.isArray(files) ? files : [files];
		try {
			const outcome = await startImport(collectionId, paths);
			importFailures = outcome.failures;
		} catch (e) {
			importError = describeError(e);
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

	function undeleteDeletedDocument(documentId: string) {
		undeleteDocument(collectionId, documentId).catch((e) => {
			console.error('Failed to undelete document:', e);
		});
	}
</script>

<Page>
	{#snippet header()}
		<CollectionHeader collection={data.collection} active="documents" />
	{/snippet}

	{#if importFailures.length > 0}
		<ImportFailures failures={importFailures} />
	{/if}

	{#if importError}
		<Alert>{importError}</Alert>
	{/if}

	{#if pipelineProgress}
		<PipelineProgress progress={pipelineProgress} />
	{/if}

	{#if documents.length === 0 && processing}
		<EmptyState
			title="Processing documents…"
			description="Documents will appear here as they finish importing."
		/>
	{:else if documents.length === 0 && deletedDocuments.length > 0}
		<EmptyState
			title="No active documents"
			description="Every document in this collection is in Deleted items."
		/>
	{:else if documents.length === 0}
		<EmptyState
			title="No documents yet"
			description="Documents you import or that arrive from colleagues syncing this collection will appear here as they become available."
		>
			{#snippet action()}
				<Button variant="solid" onclick={importPdf}>Import documents</Button>
			{/snippet}
		</EmptyState>
	{:else}
		<div>
			<Button variant="solid" onclick={importPdf} disabled={processing}
				>Import documents</Button
			>
		</div>

		<List>
			{#each documents as doc (doc.id)}
				<DocumentRow
					document={doc}
					href={resolve(`collections/${collectionId}/documents/${doc.id}`)}
					onDelete={(d) => (deletingDocument = d)}
				/>
			{/each}
		</List>
	{/if}

	{#if deletedDocuments.length > 0}
		<hr class="border-neutral-200" />
		<PageSection title="Deleted items" id="deleted-items-heading">
			<List labelledBy="deleted-items-heading">
				{#each deletedDocuments as doc (doc.id)}
					<DeletedDocumentRow
						document={doc}
						onRestore={undeleteDeletedDocument}
					/>
				{/each}
			</List>
		</PageSection>
	{/if}
</Page>

{#if deletingDocument}
	<ConfirmDialog
		title="Delete document"
		confirmLabel="Delete document"
		message={`Delete "${deletingDocument.name}"? Everyone syncing this collection will stop seeing it in the active list and search. You can restore it from Deleted items.`}
		onConfirm={confirmDeleteDocument}
		onCancel={() => (deletingDocument = null)}
	/>
{/if}
