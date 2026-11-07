<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';

	let { children } = $props();

	type Tab = { id: string; label: string; href: 'files' | 'settings' };
	const tabs: Tab[] = [
		{ id: 'files', label: 'Files', href: 'files' },
		{ id: 'settings', label: 'Settings', href: 'settings' },
	];

	const currentTab = $derived.by(() => {
		// A collection's chats live under Files.
		const section = page.url.pathname.split('/')[1] || 'files';
		return section === 'research' ? 'files' : section;
	});
</script>

<main class="flex h-screen flex-col bg-surface-dim text-neutral-800">
	<nav class="flex bg-neutral-700">
		{#each tabs as tab (tab.id)}
			<a
				href={resolve(tab.href)}
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
