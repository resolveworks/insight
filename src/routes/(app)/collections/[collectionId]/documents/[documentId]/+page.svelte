<script lang="ts">
	import { resolve } from '$app/paths';
	import { onMount } from 'svelte';
	import ButtonLink from '#lib/components/ButtonLink.svelte';
	import Card from '#lib/components/Card.svelte';
	import DescriptionList from '#lib/components/DescriptionList.svelte';
	import DescriptionRow from '#lib/components/DescriptionRow.svelte';
	import DocumentText from '#lib/components/DocumentText.svelte';
	import Icon from '#lib/components/Icon.svelte';
	import Page from '#lib/components/Page.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import PageSection from '#lib/components/PageSection.svelte';
	import TagList from '#lib/components/TagList.svelte';
	import TextChunks from '#lib/components/TextChunks.svelte';
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
	<ButtonLink
		href={resolve(`collections/${collectionId}/documents`)}
		variant="outline"
		size="sm"
	>
		<Icon name="arrow-left" size="sm" />
		Back to documents
	</ButtonLink>
{/snippet}

<Page measure="prose">
	{#snippet header()}
		<PageHeader title={data.document.name}>
			{#snippet eyebrow()}
				{@render backLink()}
			{/snippet}
		</PageHeader>
	{/snippet}

	<Card>
		<DescriptionList>
			<DescriptionRow label="ID" mono>{data.document.id}</DescriptionRow>
			<DescriptionRow label="Type">{data.document.file_type}</DescriptionRow>
			{#if data.document.page_count !== null}
				<DescriptionRow label="Pages">{data.document.page_count}</DescriptionRow
				>
			{/if}
			<DescriptionRow label="Created">
				{formatTimestamp(data.document.created_at)}
			</DescriptionRow>
			<DescriptionRow label="Tags">
				<TagList tags={data.document.tags} />
			</DescriptionRow>
		</DescriptionList>
	</Card>

	<PageSection title="Content">
		<DocumentText {content} error={contentError} loading={loadingContent} />
	</PageSection>

	<TextChunks
		bind:open={chunksOpen}
		{chunks}
		error={chunksError}
		loading={loadingChunks}
	/>
</Page>
