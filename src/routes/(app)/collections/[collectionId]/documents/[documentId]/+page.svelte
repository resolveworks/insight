<script lang="ts">
	import { resolve } from '$app/paths';
	import { onMount } from 'svelte';
	import Alert from '#lib/components/Alert.svelte';
	import BackLink from '#lib/components/BackLink.svelte';
	import Card from '#lib/components/Card.svelte';
	import DescriptionList from '#lib/components/DescriptionList.svelte';
	import DescriptionRow from '#lib/components/DescriptionRow.svelte';
	import Disclosure from '#lib/components/Disclosure.svelte';
	import LoadingSpinner from '#lib/components/LoadingSpinner.svelte';
	import Page from '#lib/components/Page.svelte';
	import PageContent from '#lib/components/PageContent.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import SectionHeading from '#lib/components/SectionHeading.svelte';
	import { getDocumentChunks, getDocumentText } from '#lib/api/document.js';
	import { describeError } from '#lib/api/provider.js';
	import { formatTimestamp } from '#lib/format.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	let content = $state<string | null>(null);
	let chunks = $state<string[] | null>(null);
	let loadingContent = $state(false);
	let loadingChunks = $state(false);
	let chunksOpen = $state(false);
	let chunksError = $state<string | null>(null);
	let contentError = $state<string | null>(null);

	const collectionId = $derived(data.collection.id);
	const documentId = $derived(data.document.id);

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
		// Load document content — failure here is non-critical;
		// the document was just imported and extraction may still be running.
		loadingContent = true;
		try {
			content = await getDocumentText(collectionId, documentId);
		} catch (e) {
			contentError = describeError(e);
		} finally {
			loadingContent = false;
		}
	});
</script>

{#snippet backLink()}
	<BackLink href={resolve(`collections/${collectionId}/documents`)}>
		Back to documents
	</BackLink>
{/snippet}

<Page>
	{#snippet header()}
		<PageHeader title={data.document.name}>
			{#snippet eyebrow()}
				{@render backLink()}
			{/snippet}
		</PageHeader>
	{/snippet}

	<PageContent width="prose">
		<Card>
			<DescriptionList>
				<DescriptionRow label="ID" mono>{data.document.id}</DescriptionRow>
				<DescriptionRow label="Type">{data.document.file_type}</DescriptionRow>
				{#if data.document.page_count !== null}
					<DescriptionRow label="Pages"
						>{data.document.page_count}</DescriptionRow
					>
				{/if}
				<DescriptionRow label="Created">
					{formatTimestamp(data.document.created_at)}
				</DescriptionRow>
				<DescriptionRow label="Tags">
					{#if data.document.tags.length > 0}
						<div class="flex flex-wrap gap-1">
							{#each data.document.tags as tag (tag)}
								<span
									class="rounded-sm bg-neutral-100 px-2 py-0.5 text-xs text-neutral-700"
									>{tag}</span
								>
							{/each}
						</div>
					{:else}
						<span class="italic text-neutral-400">(none)</span>
					{/if}
				</DescriptionRow>
			</DescriptionList>
		</Card>

		<!-- Document Content -->
		<div>
			<div class="mb-3">
				<SectionHeading title="Content" />
			</div>
			{#if loadingContent}
				<LoadingSpinner size="sm" ariaLabel="Loading content" />
			{:else if contentError}
				<Alert variant="warning">{contentError}</Alert>
			{:else if content}
				<div class="max-h-96 overflow-y-auto">
					<Card padding="sm">
						<pre
							class="whitespace-pre-wrap font-sans text-base leading-relaxed text-neutral-800">{content}</pre>
					</Card>
				</div>
			{:else}
				<p class="text-sm italic text-neutral-400">No content available</p>
			{/if}
		</div>

		<!-- Text Chunks -->
		<div>
			<Disclosure bind:open={chunksOpen}>
				{#snippet summary()}
					<SectionHeading title="Text Chunks" />
					{#if chunks}
						<span class="text-xs font-normal text-neutral-400"
							>({chunks.length} chunks)</span
						>
					{/if}
				{/snippet}

				<div class="mt-3">
					{#if loadingChunks}
						<LoadingSpinner size="sm" ariaLabel="Loading chunks" />
					{:else if chunksError}
						<Alert variant="warning">{chunksError}</Alert>
					{:else if chunks && chunks.length > 0}
						<div class="space-y-3">
							{#each chunks as chunk, i (i)}
								<Card padding="sm">
									<div
										class="mb-2 flex items-center justify-between text-xs text-neutral-400"
									>
										<span>Chunk {i + 1}</span>
										<span>{chunk.length} chars</span>
									</div>
									<p
										class="whitespace-pre-wrap break-words font-sans text-sm leading-relaxed text-neutral-700"
									>
										{chunk}
									</p>
								</Card>
							{/each}
						</div>
					{:else}
						<p class="text-sm italic text-neutral-400">No chunks generated</p>
					{/if}
				</div>
			</Disclosure>
		</div>
	</PageContent>
</Page>
