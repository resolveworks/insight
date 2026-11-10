<script lang="ts">
	import { resolve } from '$app/paths';
	import BackLink from './BackLink.svelte';
	import PageHeader from './PageHeader.svelte';
	import TabNav from './TabNav.svelte';

	type Section = 'research' | 'documents' | 'settings';

	interface Props {
		collection: { id: string; name: string };
		active: Section;
	}

	let { collection, active }: Props = $props();

	const sections = [
		{ id: 'research', label: 'Research' },
		{ id: 'documents', label: 'Documents' },
		{ id: 'settings', label: 'Settings' },
	].map((section) => ({
		...section,
		href: resolve(`collections/${collection.id}/${section.id}`),
	}));
</script>

<PageHeader title={collection.name}>
	{#snippet eyebrow()}
		<BackLink href={resolve('collections')}>Back to collections</BackLink>
	{/snippet}
	{#snippet tabs()}
		<TabNav label="Collection sections" items={sections} activeId={active} />
	{/snippet}
</PageHeader>
