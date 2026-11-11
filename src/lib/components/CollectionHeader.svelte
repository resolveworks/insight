<script lang="ts">
	import { resolve } from '$app/paths';
	import PageHeader from './PageHeader.svelte';
	import TabNav from './TabNav.svelte';

	type Section = 'research' | 'documents' | 'settings';

	interface Props {
		collection: { id: string; name: string };
		active: Section;
		compact?: boolean;
	}

	let { collection, active, compact = false }: Props = $props();

	const sections = [
		{ id: 'research', label: 'Research' },
		{ id: 'documents', label: 'Documents' },
		{ id: 'settings', label: 'Settings' },
	].map((section) => ({
		...section,
		href: resolve(`collections/${collection.id}/${section.id}`),
	}));
</script>

{#if compact}
	<header class="border-b border-neutral-200 bg-white px-6 py-3">
		<div class="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
			<div class="flex min-w-0 flex-1 items-center gap-2">
				<a
					href={resolve('collections')}
					class="shrink-0 text-sm font-medium text-neutral-500 transition-colors hover:text-neutral-800 focus-visible:focus-ring"
				>
					Back to collections
				</a>
				<span aria-hidden="true" class="shrink-0 text-neutral-300">/</span>
				<span class="truncate text-sm font-medium text-neutral-800">
					{collection.name}
				</span>
			</div>
			<div class="max-w-full shrink-0 overflow-x-auto">
				<TabNav
					label="Collection sections"
					items={sections}
					activeId={active}
				/>
			</div>
		</div>
	</header>
{:else}
	<PageHeader
		title={collection.name}
		back={{ href: resolve('collections'), label: 'Back to collections' }}
	>
		{#snippet tabs()}
			<TabNav label="Collection sections" items={sections} activeId={active} />
		{/snippet}
	</PageHeader>
{/if}
