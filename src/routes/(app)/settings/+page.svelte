<script lang="ts">
	import { resolve } from '$app/paths';
	import BackLink from '#lib/components/BackLink.svelte';
	import CardSection from '#lib/components/CardSection.svelte';
	import Page from '#lib/components/Page.svelte';
	import PageContent from '#lib/components/PageContent.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import ProviderSelector from '#lib/components/ProviderSelector.svelte';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
</script>

<Page>
	{#snippet header()}
		<PageHeader title="Settings">
			{#snippet eyebrow()}
				<BackLink href={resolve('collections')}>Back to collections</BackLink>
			{/snippet}
		</PageHeader>
	{/snippet}
	<PageContent width="prose">
		<CardSection
			title="Language Model"
			description="Choose a chat provider from the registry. Only the conversation is sent to the provider — documents never leave your machine."
		>
			<ProviderSelector
				provider={data.provider}
				providerError={data.providerError}
			/>
		</CardSection>
	</PageContent>
</Page>
