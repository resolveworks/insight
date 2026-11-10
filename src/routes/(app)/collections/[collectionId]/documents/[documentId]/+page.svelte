<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { onMount } from 'svelte';
	import Alert from '#lib/components/Alert.svelte';
	import BackLink from '#lib/components/BackLink.svelte';
	import Disclosure from '#lib/components/Disclosure.svelte';
	import Icon from '#lib/components/Icon.svelte';
	import LoadingSpinner from '#lib/components/LoadingSpinner.svelte';
	import PageContent from '#lib/components/PageContent.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import SectionHeading from '#lib/components/SectionHeading.svelte';
	import {
		getDocument,
		getDocumentChunks,
		getDocumentText,
	} from '#lib/api/document.js';
	import { describeError } from '#lib/api/provider.js';
	import { formatTimestamp } from '#lib/format.js';
	import type { Document } from '#lib/types/documents.js';

	let document = $state<Document | null>(null);
	let content = $state<string | null>(null);
	let chunks = $state<string[] | null>(null);
	let loading = $state(true);
	let loadingContent = $state(false);
	let loadingChunks = $state(false);
	let chunksOpen = $state(false);
	let chunksError = $state<string | null>(null);
	let error = $state<string | null>(null);
	let contentError = $state<string | null>(null);

	const collectionId = $derived(page.params.collectionId!);
	const documentId = $derived(page.params.documentId!);

	interface MetadataRow {
		label: string;
		value: string;
		mono?: boolean;
	}

	const metadataRows: MetadataRow[] = $derived(
		document
			? [
					{ label: 'ID', value: document.id, mono: true },
					{ label: 'Type', value: document.file_type },
					...(document.page_count !== null
						? [{ label: 'Pages', value: `${document.page_count}` }]
						: []),
					{ label: 'Created', value: formatTimestamp(document.created_at) },
				]
			: [],
	);

	async function loadChunks() {
		if (chunks !== null || loadingChunks) return;

		loadingChunks = true;
		chunksError = null;

		try {
			chunks = await getDocumentChunks(collectionId, documentId);
		} catch (e) {
			chunksError = describeError(e);
		} finally {
			loadingChunks = false;
		}
	}

	$effect(() => {
		if (chunksOpen && chunks === null) {
			void loadChunks();
		}
	});

	onMount(async () => {
		try {
			document = await getDocument(collectionId, documentId);
		} catch (e) {
			error = describeError(e);
			loading = false;
			return;
		}

		// Load document content — failure here is non-critical;
		// the document was just imported and extraction may still be running.
		loadingContent = true;
		try {
			content = await getDocumentText(collectionId, documentId);
		} catch (e) {
			contentError = describeError(e);
		}

		loading = false;
		loadingContent = false;
	});
</script>

{#snippet backLink()}
	<BackLink href={resolve(`collections/${collectionId}/documents`)}>
		Back to documents
	</BackLink>
{/snippet}

{#snippet preBlock(text: string)}
	<pre
		class="whitespace-pre-wrap font-sans text-sm leading-relaxed text-neutral-700">{text}</pre>
{/snippet}

<div class="flex h-full flex-col bg-surface">
	{#if document}
		<PageHeader title={document.name}>
			{#snippet eyebrow()}
				{@render backLink()}
			{/snippet}
		</PageHeader>
	{:else}
		<PageContent>
			<div class="mb-4">
				{@render backLink()}
			</div>
			{#if loading}
				<LoadingSpinner size="lg" ariaLabel="Loading document" />
			{:else if error}
				<Alert>{error}</Alert>
			{/if}
		</PageContent>
	{/if}

	{#if document}
		<PageContent width="prose">
			<div class="card">
				<table class="w-full">
					<tbody class="divide-y divide-line">
						{#each metadataRows as row (row.label)}
							<tr>
								<td class="px-4 py-3 text-sm font-medium text-muted">
									{row.label}
								</td>
								<td
									class="px-4 py-3 text-sm text-neutral-700"
									class:font-mono={row.mono}
								>
									{row.value}
								</td>
							</tr>
						{/each}
						<tr>
							<td class="px-4 py-3 text-sm font-medium text-muted">Tags</td>
							<td class="px-4 py-3 text-sm text-neutral-700">
								{#if document.tags.length > 0}
									<div class="flex flex-wrap gap-1">
										{#each document.tags as tag (tag)}
											<span
												class="rounded-sm bg-surface-dim px-2 py-0.5 text-xs text-neutral-700"
												>{tag}</span
											>
										{/each}
									</div>
								{:else}
									<span class="text-sm italic text-subtle">(none)</span>
								{/if}
							</td>
						</tr>
					</tbody>
				</table>
			</div>

			<!-- Document Content -->
			<div class="mt-6">
				<SectionHeading title="Content" class="mb-3" />
				{#if loadingContent}
					<LoadingSpinner size="sm" ariaLabel="Loading content" />
				{:else if contentError}
					<Alert variant="warning">{contentError}</Alert>
				{:else if content}
					<div class="max-h-96 overflow-y-auto card p-4">
						{@render preBlock(content)}
					</div>
				{:else}
					<p class="text-sm italic text-subtle">No content available</p>
				{/if}
			</div>

			<!-- Text Chunks -->
			<Disclosure bind:open={chunksOpen} class="mt-6">
				{#snippet summary()}
					<div class="flex items-center gap-2">
						<Icon
							name="chevron-right"
							size="sm"
							class="transition-transform group-open:rotate-90"
						/>
						<SectionHeading title="Text Chunks" />
						{#if chunks}
							<span class="text-xs font-normal text-subtle"
								>({chunks.length} chunks)</span
							>
						{/if}
					</div>
				{/snippet}

				<div class="mt-3">
					{#if loadingChunks}
						<LoadingSpinner size="sm" ariaLabel="Loading chunks" />
					{:else if chunksError}
						<Alert variant="warning">{chunksError}</Alert>
					{:else if chunks && chunks.length > 0}
						<div class="space-y-3">
							{#each chunks as chunk, i (i)}
								<div class="card p-4">
									<div
										class="mb-2 flex items-center justify-between text-xs text-subtle"
									>
										<span>Chunk {i + 1}</span>
										<span>{chunk.length} chars</span>
									</div>
									{@render preBlock(chunk)}
								</div>
							{/each}
						</div>
					{:else}
						<p class="text-sm italic text-subtle">No chunks generated</p>
					{/if}
				</div>
			</Disclosure>
		</PageContent>
	{/if}
</div>
