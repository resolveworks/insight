<script lang="ts">
	import { resolve } from '$app/paths';
	import CardSection from '#lib/components/CardSection.svelte';
	import Page from '#lib/components/Page.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import ProviderSelector from '#lib/components/ProviderSelector.svelte';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
</script>

<Page measure="prose">
	{#snippet header()}
		<PageHeader
			title="Settings"
			back={{ href: resolve('collections'), label: 'Back to collections' }}
		/>
	{/snippet}
	<CardSection
		title="Language Model"
		description="Choose a chat provider from the registry. Only the conversation is sent to the provider — documents never leave your machine."
	>
		<ProviderSelector
			provider={data.provider}
			providerError={data.providerError}
		/>
	</CardSection>
</Page>
