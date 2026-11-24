<script lang="ts">
	import { page } from '$app/state';
	import { replaceState } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { onMount, untrack } from 'svelte';
	import Alert from '#lib/components/Alert.svelte';
	import Button from '#lib/components/Button.svelte';
	import ButtonLink from '#lib/components/ButtonLink.svelte';
	import DocumentSearch from '#lib/components/DocumentSearch.svelte';
	import Icon from '#lib/components/Icon.svelte';
	import DocumentText from '#lib/components/DocumentText.svelte';
	import Page from '#lib/components/Page.svelte';
	import TagList from '#lib/components/TagList.svelte';
	import {
		getDocumentText,
		openDocumentOriginal,
		searchDocument,
	} from '#lib/api/document.js';
	import { describeError } from '#lib/api/provider.js';
	import { formatTimestamp } from '#lib/format.js';
	import type { DocumentSearchResults } from '#lib/types/documents.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	function requestedHit(url: typeof page.url): number {
		const raw = url.searchParams.get('hit');
		if (raw === null) return 1;
		const parsed = Number(raw);
		return Number.isInteger(parsed) && parsed > 0 ? parsed : 1;
	}

	let content = $state<string | null>(null);
	let loadingContent = $state(false);
	let contentError = $state<string | null>(null);
	let query = $state(page.url.searchParams.get('q') ?? '');
	let results = $state<DocumentSearchResults | null>(null);
	let activeHit = $state(0);
	let searching = $state(false);
	let searchError = $state<string | null>(null);
	let openingOriginal = $state(false);
	let openOriginalError = $state<string | null>(null);

	const collectionId = $derived(data.collection.id);
	const documentId = $derived(data.document.id);

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
		});
	}

	async function openOriginal() {
		openingOriginal = true;
		openOriginalError = null;
		try {
			await openDocumentOriginal(collectionId, documentId);
		} catch (e) {
			openOriginalError = `Failed to open original: ${describeError(e)}`;
		} finally {
			openingOriginal = false;
		}
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

<Page measure="prose">
	<div>
		<div class="flex flex-wrap items-center gap-2">
			<ButtonLink
				href={resolve(`collections/${collectionId}/documents`)}
				variant="outline"
				size="sm"
			>
				<Icon name="arrow-left" size="sm" />
				Back to documents
			</ButtonLink>
			<Button
				variant="outline"
				size="sm"
				disabled={openingOriginal}
				onclick={openOriginal}
			>
				Open original
			</Button>
		</div>
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

	{#if openOriginalError}
		<Alert>{openOriginalError}</Alert>
	{/if}

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

	<DocumentText
		{content}
		error={contentError}
		loading={loadingContent}
		matches={results?.matches ?? []}
		{activeHit}
		visible={true}
	/>
</Page>
