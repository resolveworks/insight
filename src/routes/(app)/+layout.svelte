<script lang="ts">
	import { page } from '$app/stores';
	import { resolve } from '$app/paths';

	let { children } = $props();

	type Tab = { id: string; label: string; href: string };
	const tabs: Tab[] = [
		{ id: 'research', label: 'Research', href: '/research' },
		{ id: 'files', label: 'Files', href: '/files' },
		{ id: 'settings', label: 'Settings', href: '/settings' },
	];

	const currentTab = $derived($page.url.pathname.split('/')[1] || 'research');
</script>

<main class="flex h-screen flex-col bg-surface-dim text-neutral-800">
	<nav class="flex bg-neutral-700">
		{#each tabs as tab (tab.id)}
			<a
				href={resolve(tab.href as '/')}
				class="px-6 py-3 text-sm font-medium transition-colors {currentTab ===
				tab.id
					? 'border-b-2 border-primary-500 text-surface'
					: 'text-neutral-300 hover:text-surface'}"
			>
				{tab.label}
			</a>
		{/each}
	</nav>
	<div class="flex-1 overflow-hidden">
		{@render children()}
	</div>
</main>
