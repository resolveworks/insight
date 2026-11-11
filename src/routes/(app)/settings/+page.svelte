<script lang="ts">
	import { resolve } from '$app/paths';
	import ButtonLink from '#lib/components/ButtonLink.svelte';
	import CardSection from '#lib/components/CardSection.svelte';
	import Icon from '#lib/components/Icon.svelte';
	import Page from '#lib/components/Page.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import ProviderSelector from '#lib/components/ProviderSelector.svelte';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
</script>

<Page measure="prose">
	{#snippet header()}
		<PageHeader title="Settings">
			{#snippet eyebrow()}
				<ButtonLink href={resolve('collections')} variant="outline" size="sm">
					<Icon name="arrow-left" size="sm" />
					Back to collections
				</ButtonLink>
			{/snippet}
		</PageHeader>
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
