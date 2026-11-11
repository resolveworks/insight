<script lang="ts">
	import { resolve } from '$app/paths';
	import Alert from './Alert.svelte';
	import ButtonLink from './ButtonLink.svelte';
	import Button from './Button.svelte';
	import Icon from './Icon.svelte';
	import EmptyState from './EmptyState.svelte';

	interface Props {
		/** A missing collection, rather than a failed load. */
		notFound?: boolean;
		message: string;
		onRetry: () => void;
	}

	let { notFound = false, message, onRetry }: Props = $props();
</script>

<div class="flex h-full items-center justify-center p-6">
	{#if notFound}
		<EmptyState title="Collection not found">
			{#snippet action()}
				<ButtonLink href={resolve('collections')} variant="outline" size="sm">
					<Icon name="arrow-left" size="sm" />
					Back to collections
				</ButtonLink>
			{/snippet}
		</EmptyState>
	{:else}
		<div class="w-full max-w-md">
			<Alert>
				{message}
				{#snippet action()}
					<Button variant="solid" onclick={onRetry}>Retry</Button>
				{/snippet}
			</Alert>
		</div>
	{/if}
</div>
