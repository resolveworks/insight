<script lang="ts">
	import { page } from '$app/state';
	import CollectionHeader, {
		type Section,
	} from '#lib/components/CollectionHeader.svelte';
	import type { LayoutProps } from './$types';

	let { data, children }: LayoutProps = $props();

	const sectionByRoute: Record<string, Section> = {
		'/(app)/collections/[collectionId]/research': 'research',
		'/(app)/collections/[collectionId]/documents': 'documents',
		'/(app)/collections/[collectionId]/documents/[documentId]': 'documents',
		'/(app)/collections/[collectionId]/settings': 'settings',
	};

	const active = $derived.by((): Section => {
		const section = sectionByRoute[page.route.id ?? ''];
		if (section === undefined) {
			throw new Error(`Unknown collection route: ${page.route.id}`);
		}
		return section;
	});
</script>

<div class="flex h-full flex-col">
	<CollectionHeader collection={data.collection} {active} />
	<div class="min-h-0 flex-1">
		{@render children()}
	</div>
</div>
