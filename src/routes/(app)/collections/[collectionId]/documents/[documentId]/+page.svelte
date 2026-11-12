<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { onMount } from 'svelte';
	import Alert from '#lib/components/Alert.svelte';
	import ButtonLink from '#lib/components/ButtonLink.svelte';
	import Icon from '#lib/components/Icon.svelte';
	import DocumentText from '#lib/components/DocumentText.svelte';
	import Page from '#lib/components/Page.svelte';
	import TagList from '#lib/components/TagList.svelte';
	import { getDocumentText } from '#lib/api/document.js';
	import { describeError } from '#lib/api/provider.js';
	import { formatTimestamp } from '#lib/format.js';
	import { rendererFor } from '#lib/renderers/index.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	type Tab = 'source' | 'text';

	let content = $state<string | null>(null);
	let loadingContent = $state(false);
	let contentError = $state<string | null>(null);
	let activeTab = $state<Tab>('source');

	const collectionId = $derived(data.collection.id);
	const documentId = $derived(data.document.id);
	const renderer = $derived(rendererFor(data.document.file_type));

	const requestedPage = $derived.by(() => {
		const raw = page.url.searchParams.get('page');
		if (raw === null) return null;
		const parsed = Number(raw);
		return Number.isInteger(parsed) && parsed > 0 ? parsed : null;
	});

	let tabDocumentId: string | undefined;

	$effect(() => {
		if (documentId !== tabDocumentId || requestedPage !== null) {
			tabDocumentId = documentId;
			activeTab = 'source';
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

<Page measure={renderer ? 'wide' : 'prose'}>
	<div>
		<ButtonLink
			href={resolve(`collections/${collectionId}/documents`)}
			variant="outline"
			size="sm"
		>
			<Icon name="arrow-left" size="sm" />
			Back to documents
		</ButtonLink>
		<h2 class="truncate text-xl font-semibold text-neutral-800">
			{data.document.name}
		</h2>
		<div
			class="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-neutral-500"
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
	</div>

	{#if data.document.partially_indexed === true}
		<Alert variant="warning">
			<p class="font-medium">Partially searchable</p>
			<p>
				Text near the end of this document is not included in search because it
				exceeds the indexing limit. The full document is still available.
			</p>
		</Alert>
	{/if}

	{#if renderer}
		<div role="tablist" class="flex gap-1 border-b border-neutral-200">
			<button
				type="button"
				role="tab"
				id="source-tab"
				aria-selected={activeTab === 'source'}
				aria-controls="source-panel"
				class="-mb-px border-b-2 px-4 py-2 text-sm font-medium {activeTab ===
				'source'
					? 'border-neutral-800 text-neutral-800'
					: 'border-transparent text-neutral-500 hover:text-neutral-700'}"
				onclick={() => (activeTab = 'source')}
			>
				Source
			</button>
			<button
				type="button"
				role="tab"
				id="text-tab"
				aria-selected={activeTab === 'text'}
				aria-controls="text-panel"
				class="-mb-px border-b-2 px-4 py-2 text-sm font-medium {activeTab ===
				'text'
					? 'border-neutral-800 text-neutral-800'
					: 'border-transparent text-neutral-500 hover:text-neutral-700'}"
				onclick={() => (activeTab = 'text')}
			>
				Text
			</button>
		</div>

		<div
			role="tabpanel"
			id="source-panel"
			aria-labelledby="source-tab"
			hidden={activeTab !== 'source'}
		>
			{#await renderer.load() then Renderer}
				<Renderer {collectionId} {documentId} page={requestedPage} />
			{/await}
		</div>

		<div
			role="tabpanel"
			id="text-panel"
			aria-labelledby="text-tab"
			hidden={activeTab !== 'text'}
			class="mx-auto max-w-2xl"
		>
			<DocumentText {content} error={contentError} loading={loadingContent} />
		</div>
	{:else}
		<DocumentText {content} error={contentError} loading={loadingContent} />
	{/if}
</Page>
