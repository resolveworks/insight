<script lang="ts">
	import { resolve } from '$app/paths';
	import ButtonLink from './ButtonLink.svelte';
	import Icon from './Icon.svelte';
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
		<ButtonLink href={resolve('collections')} variant="outline" size="sm">
			<Icon name="arrow-left" size="sm" />
			Back to collections
		</ButtonLink>
	{/snippet}
	{#snippet tabs()}
		<TabNav label="Collection sections" items={sections} activeId={active} />
	{/snippet}
</PageHeader>
