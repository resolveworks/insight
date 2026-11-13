<script lang="ts">
	import { page } from '$app/state';
	import { replaceState } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { onMount, untrack } from 'svelte';
	import Alert from '#lib/components/Alert.svelte';
	import ButtonLink from '#lib/components/ButtonLink.svelte';
	import DocumentSearch from '#lib/components/DocumentSearch.svelte';
	import Icon from '#lib/components/Icon.svelte';
	import DocumentText from '#lib/components/DocumentText.svelte';
	import Page from '#lib/components/Page.svelte';
	import TagList from '#lib/components/TagList.svelte';
	import { getDocumentText, searchDocument } from '#lib/api/document.js';
	import { describeError } from '#lib/api/provider.js';
	import { formatTimestamp } from '#lib/format.js';
	import { rendererFor } from '#lib/renderers/index.js';
	import type { DocumentSearchResults } from '#lib/types/documents.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	type Tab = 'source' | 'text';
	type PageUrl = typeof page.url;

	function requestedHit(url: PageUrl): number {
		const raw = url.searchParams.get('hit');
		if (raw === null) return 1;
		const parsed = Number(raw);
		return Number.isInteger(parsed) && parsed > 0 ? parsed : 1;
	}

	function requestedTab(url: PageUrl): Tab {
		return url.searchParams.get('view') === 'text' ? 'text' : 'source';
	}

	let content = $state<string | null>(null);
	let loadingContent = $state(false);
	let contentError = $state<string | null>(null);
	let activeTab = $state<Tab>(requestedTab(page.url));
	let query = $state(page.url.searchParams.get('q') ?? '');
	let results = $state<DocumentSearchResults | null>(null);
	let activeHit = $state(0);
	let searching = $state(false);
	let searchError = $state<string | null>(null);

	const collectionId = $derived(data.collection.id);
	const documentId = $derived(data.document.id);
	const renderer = $derived(rendererFor(data.document.file_type));

	const requestedPage = $derived.by(() => {
		const raw = page.url.searchParams.get('page');
		if (raw === null) return null;
		const parsed = Number(raw);
		return Number.isInteger(parsed) && parsed > 0 ? parsed : null;
	});

	let syncedUrl: string | undefined;

	$effect(() => {
		const href = page.url.href;
		if (href === syncedUrl) return;
		syncedUrl = href;
		activeTab = requestedTab(page.url);
	});

	let searchRequest = 0;
	let searchTimer: ReturnType<typeof setTimeout> | undefined;

	function cancelSearch() {
		searchRequest += 1;
		if (searchTimer !== undefined) {
			clearTimeout(searchTimer);
			searchTimer = undefined;
		}
	}

	async function runSearch(value: string, hit: number) {
		const request = ++searchRequest;
		results = null;
		searchError = null;
		searching = true;
		try {
			const found = await searchDocument(collectionId, documentId, value);
			if (request !== searchRequest) return;
			results = found;
			activeHit =
				found.matches.length === 0 || hit > found.matches.length ? 0 : hit - 1;
		} catch (e) {
			if (request !== searchRequest) return;
			searchError = describeError(e);
		} finally {
			if (request === searchRequest) searching = false;
		}
	}

	function updateUrl(change: (url: URL) => void) {
		const url = new URL(page.url.href);
		change(url);
		// The current route with its query replaced; resolve() cannot type a runtime query.
		// eslint-disable-next-line svelte/no-navigation-without-resolve
		void replaceState(url, page.state);
	}

	let searchDocumentId: string | undefined;

	$effect(() => {
		const id = documentId;
		if (id === searchDocumentId) return;
		searchDocumentId = id;

		const url = untrack(() => page.url);
		const value = url.searchParams.get('q') ?? '';
		cancelSearch();
		query = value;
		results = null;
		activeHit = 0;
		searchError = null;
		searching = false;
		if (value.trim() !== '') void runSearch(value, requestedHit(url));
	});

	$effect(() => {
		return () => {
			cancelSearch();
		};
	});

	function handleQueryChange(value: string) {
		query = value;
		activeHit = 0;
		cancelSearch();
		results = null;
		searchError = null;
		searching = false;

		updateUrl((url) => {
			if (value === '') url.searchParams.delete('q');
			else url.searchParams.set('q', value);
			url.searchParams.delete('hit');
			url.searchParams.delete('page');
		});

		if (value.trim() === '') return;

		searchTimer = setTimeout(() => {
			searchTimer = undefined;
			void runSearch(value, 1);
		}, 200);
	}

	function clearSearch() {
		handleQueryChange('');
	}

	function moveHit(step: number) {
		const matches = results?.matches ?? [];
		if (matches.length === 0) return;
		activeHit = (activeHit + step + matches.length) % matches.length;
		updateUrl((url) => {
			url.searchParams.set('hit', String(activeHit + 1));
			url.searchParams.delete('page');
		});
	}

	function selectTab(tab: Tab) {
		activeTab = tab;
		updateUrl((url) => {
			if (tab === 'text') url.searchParams.set('view', 'text');
			else url.searchParams.delete('view');
		});
	}

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

	<DocumentSearch
		bind:query
		{results}
		{activeHit}
		{searching}
		error={searchError}
		disabled={data.document.partially_indexed === null}
		onquerychange={handleQueryChange}
		onprevious={() => moveHit(-1)}
		onnext={() => moveHit(1)}
		onclear={clearSearch}
	/>

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
				onclick={() => selectTab('source')}
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
				onclick={() => selectTab('text')}
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
				<Renderer
					{collectionId}
					{documentId}
					page={requestedPage}
					{query}
					{activeHit}
				/>
			{/await}
		</div>

		<div
			role="tabpanel"
			id="text-panel"
			aria-labelledby="text-tab"
			hidden={activeTab !== 'text'}
			class="mx-auto max-w-2xl"
		>
			<DocumentText
				{content}
				error={contentError}
				loading={loadingContent}
				matches={results?.matches ?? []}
				{activeHit}
				visible={activeTab === 'text'}
			/>
		</div>
	{:else}
		<DocumentText
			{content}
			error={contentError}
			loading={loadingContent}
			matches={results?.matches ?? []}
			{activeHit}
			visible={true}
		/>
	{/if}
</Page>
