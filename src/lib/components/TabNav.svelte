<script lang="ts">
	interface Item {
		id: string;
		label: string;
		href: string;
	}

	interface Props {
		label: string;
		items: Item[];
		activeId: string;
	}

	let { label, items, activeId }: Props = $props();
</script>

<!-- Callers pass already-resolved paths. -->
<!-- eslint-disable svelte/no-navigation-without-resolve -->
<nav aria-label={label} class="flex gap-1">
	{#each items as item (item.id)}
		{@const active = item.id === activeId}
		<a
			href={item.href}
			class="rounded-t-md border-b-2 px-4 py-2 text-sm font-medium no-underline transition-colors hover:no-underline focus-visible:focus-ring {active
				? 'border-primary-500 text-primary-600'
				: 'border-transparent text-muted hover:text-neutral-800'}"
			aria-current={active ? 'page' : undefined}
		>
			{item.label}
		</a>
	{/each}
</nav>
<!-- eslint-enable svelte/no-navigation-without-resolve -->
