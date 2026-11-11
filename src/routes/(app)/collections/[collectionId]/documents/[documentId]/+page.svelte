<script lang="ts">
	import { resolve } from '$app/paths';
	import { onMount } from 'svelte';
	import Alert from '#lib/components/Alert.svelte';
	import DocumentText from '#lib/components/DocumentText.svelte';
	import Page from '#lib/components/Page.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import TagList from '#lib/components/TagList.svelte';
	import { getDocumentText } from '#lib/api/document.js';
	import { describeError } from '#lib/api/provider.js';
	import { formatTimestamp } from '#lib/format.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	let content = $state<string | null>(null);
	let loadingContent = $state(false);
	let contentError = $state<string | null>(null);

	const collectionId = $derived(data.collection.id);
	const documentId = $derived(data.document.id);

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

<Page measure="prose">
	{#snippet header()}
		<PageHeader
			title={data.document.name}
			back={{
				href: resolve(`collections/${collectionId}/documents`),
				label: 'Back to documents',
			}}
		>
			{#snippet subtitle()}
				<div
					class="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-neutral-500"
				>
					{#if data.document.page_count !== null}
						<span>
							{data.document.page_count}
							{data.document.page_count === 1 ? 'page' : 'pages'}
						</span>
						<span aria-hidden="true">·</span>
					{/if}
					<span>Added {formatTimestamp(data.document.created_at)}</span>
					{#if data.document.tags.length > 0}
						<span aria-hidden="true">·</span>
						<TagList tags={data.document.tags} />
					{/if}
				</div>
			{/snippet}
		</PageHeader>
	{/snippet}

	{#if data.document.partially_indexed === true}
		<Alert variant="warning">
			<p class="font-medium">Partially searchable</p>
			<p>
				Text near the end of this document is not included in search because it
				exceeds the indexing limit. The full document is still available.
			</p>
		</Alert>
	{/if}

	<DocumentText {content} error={contentError} loading={loadingContent} />
</Page>
