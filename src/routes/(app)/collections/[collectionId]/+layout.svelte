<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import BackLink from '#lib/components/BackLink.svelte';
	import Page from '#lib/components/Page.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import TabNav from '#lib/components/TabNav.svelte';
	import type { LayoutProps } from './$types';

	let { data, children }: LayoutProps = $props();

	const sectionTabs = [
		{ id: 'research', label: 'Research' },
		{ id: 'documents', label: 'Documents' },
		{ id: 'settings', label: 'Settings' },
	].map((tab) => ({
		...tab,
		href: resolve(`collections/${data.collection.id}/${tab.id}`),
	}));
	const currentTab = $derived(
		sectionTabs.find((tab) => page.route.id?.includes(`/${tab.id}`))?.id,
	);
	const isDocumentDetail = $derived(
		page.route.id?.endsWith('/documents/[documentId]') ?? false,
	);
</script>

<Page>
	{#if !isDocumentDetail}
		<PageHeader title={data.collection.name}>
			{#snippet eyebrow()}
				<BackLink href={resolve('collections')}>Back to collections</BackLink>
			{/snippet}
			{#snippet tabs()}
				<TabNav
					label="Collection sections"
					items={sectionTabs}
					activeId={currentTab ?? ''}
				/>
			{/snippet}
		</PageHeader>
	{/if}
	<div class="flex-1 overflow-hidden">
		{@render children()}
	</div>
</Page>
