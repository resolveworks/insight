<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import type { LayoutProps } from './$types';

	let { data, children }: LayoutProps = $props();

	const sectionTabs = [
		{ id: 'research', label: 'Research' },
		{ id: 'documents', label: 'Documents' },
		{ id: 'settings', label: 'Settings' },
	];
	const currentTab = $derived(page.route.id?.split('/').at(-1));
</script>

<div class="flex h-full flex-col bg-surface">
	<PageHeader eyebrow="Collection" title={data.collection.name}>
		{#snippet tabs()}
			<nav aria-label="Collection sections" class="flex gap-1">
				{#each sectionTabs as tab (tab.id)}
					<a
						href={resolve(`collections/${data.collection.id}/${tab.id}`)}
						aria-current={currentTab === tab.id ? 'page' : undefined}
						class="rounded-t-md border-b-2 px-4 py-2 text-sm font-medium transition-colors {currentTab ===
						tab.id
							? 'border-primary-500 text-primary-600'
							: 'border-transparent text-muted hover:text-neutral-800'}"
					>
						{tab.label}
					</a>
				{/each}
			</nav>
		{/snippet}
	</PageHeader>
	<div class="flex-1 overflow-hidden">
		{@render children()}
	</div>
</div>
