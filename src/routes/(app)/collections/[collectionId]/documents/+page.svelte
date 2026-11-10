<script lang="ts">
	import { resolve } from '$app/paths';
	import { listen } from '@tauri-apps/api/event';
	import { open } from '@tauri-apps/plugin-dialog';
	import { onMount } from 'svelte';
	import Alert from '#lib/components/Alert.svelte';
	import Button from '#lib/components/Button.svelte';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import EmptyState from '#lib/components/EmptyState.svelte';
	import Icon from '#lib/components/Icon.svelte';
	import IconButton from '#lib/components/IconButton.svelte';
	import List from '#lib/components/List.svelte';
	import ListRow from '#lib/components/ListRow.svelte';
	import Page from '#lib/components/Page.svelte';
	import PageContent from '#lib/components/PageContent.svelte';
	import ProgressBar from '#lib/components/ProgressBar.svelte';
	import SectionHeading from '#lib/components/SectionHeading.svelte';
	import {
		deleteDocument,
		getPipelineProgress,
		startImport,
		undeleteDocument,
	} from '#lib/api/document.js';
	import { describeError } from '#lib/api/provider.js';
	import { formatTimestamp, shortenId } from '#lib/format.js';
	import { pipelineIsActive } from '#lib/pipeline.js';
	import type {
		Document,
		ImportOutcome,
		PipelineProgress,
	} from '#lib/types/documents.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	let deletingDocument = $state<Document | null>(null);
	let pipelineProgress = $state<PipelineProgress>();
	let importFailures = $state<ImportOutcome['failures']>([]);
	let importError = $state<string | null>(null);

	const collectionId = $derived(data.collection.id);
	const documents = $derived(data.documents);
	const deletedDocuments = $derived(data.deletedDocuments);
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
		importFailures = [];
		importError = null;

		const files = await open({
			multiple: true,
			filters: [{ name: 'PDF', extensions: ['pdf'] }],
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
	<!-- Content -->
	<PageContent>
		{#if importFailures.length > 0}
			<div class="mb-6">
				<Alert variant="warning">
					<p class="font-medium">Some files could not be imported.</p>
					<ul class="mt-1 list-disc pl-5">
						{#each importFailures as failure, index (index)}
							<li>{failure.path}: {failure.message}</li>
						{/each}
					</ul>
				</Alert>
			</div>
		{/if}

		{#if importError}
			<div class="mb-6">
				<Alert>{importError}</Alert>
			</div>
		{/if}

		{#if processing && stages.length > 0}
			<div class="mb-6 grid grid-cols-3 gap-4">
				{#each stages as stage (stage.name)}
					{@const total =
						stage.data.pending +
						stage.data.active +
						stage.data.completed +
						stage.data.failed}
					{@const done = stage.data.completed + stage.data.failed}
					<ProgressBar
						label={stage.name}
						detail={`${done}/${total}`}
						value={done}
						max={total}
					/>
				{/each}
			</div>
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
					<Button onclick={importPdf}>Import PDF</Button>
				{/snippet}
			</EmptyState>
		{:else}
			<div class="mb-4 flex items-center">
				<Button onclick={importPdf} disabled={processing}>Import PDF</Button>
			</div>

			<List>
				{#each documents as doc (doc.id)}
					<ListRow interactive>
						<a
							href={resolve(`collections/${collectionId}/documents/${doc.id}`)}
							class="block min-w-0 focus-visible:focus-ring"
						>
							<span class="text-sm text-neutral-800">{doc.name}</span>
							{#if doc.page_count !== null}
								<p class="mt-1 text-xs text-muted">
									{doc.page_count} pages
								</p>
							{/if}
						</a>
						{#snippet actions()}
							<IconButton
								variant="danger"
								label="Delete document"
								onclick={() => (deletingDocument = doc)}
							>
								<Icon name="trash" size="sm" />
							</IconButton>
						{/snippet}
					</ListRow>
				{/each}
			</List>
		{/if}

		{#if deletedDocuments.length > 0}
			<section class="mt-8 border-t border-line pt-6">
				<div class="mb-3">
					<SectionHeading id="deleted-items-heading" title="Deleted items" />
				</div>
				<List labelledBy="deleted-items-heading">
					{#each deletedDocuments as doc (doc.id)}
						<ListRow>
							<p class="text-sm text-neutral-800">{doc.name}</p>
							<p class="mt-1 text-xs text-muted">
								Deleted by {shortenId(doc.deleted_by)} · {formatTimestamp(
									doc.deleted_at,
								)}
							</p>
							{#snippet actions()}
								<Button
									variant="secondary"
									size="sm"
									onclick={() => undeleteDeletedDocument(doc.id)}
								>
									Restore
								</Button>
							{/snippet}
						</ListRow>
					{/each}
				</List>
			</section>
		{/if}
	</PageContent>
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
