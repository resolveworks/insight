<script lang="ts">
	import { resolve } from '$app/paths';
	import BackLink from '#lib/components/BackLink.svelte';
	import PageContent from '#lib/components/PageContent.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import ProviderSelector from '#lib/components/ProviderSelector.svelte';
	import SectionHeading from '#lib/components/SectionHeading.svelte';
</script>

<div class="flex h-full flex-col bg-surface">
	<PageHeader title="Settings">
		{#snippet eyebrow()}
			<BackLink href={resolve('collections')}>Back to collections</BackLink>
		{/snippet}
	</PageHeader>
	<PageContent width="prose">
		<SectionHeading title="Language Model" />
		<p class="mt-2 text-sm text-muted">
			Choose a chat provider from the registry. Only the conversation is sent to
			the provider — documents never leave your machine.
		</p>
		<div class="card mt-4 p-6">
			<ProviderSelector />
		</div>
	</PageContent>
</div>
