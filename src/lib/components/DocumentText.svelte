<script lang="ts">
	import Alert from './Alert.svelte';
	import Card from './Card.svelte';
	import LoadingSpinner from './LoadingSpinner.svelte';

	interface Props {
		content: string | null;
		error: string | null;
		loading: boolean;
	}

	let { content, error, loading }: Props = $props();
</script>

{#if loading}
	<LoadingSpinner size="sm" ariaLabel="Loading content" />
{:else if error}
	<Alert variant="warning">{error}</Alert>
{:else if content}
	<div class="max-h-96 overflow-y-auto">
		<Card padding="sm">
			<pre
				class="font-sans text-base leading-relaxed whitespace-pre-wrap text-neutral-800">{content}</pre>
		</Card>
	</div>
{:else}
	<p class="text-sm text-neutral-400 italic">No content available</p>
{/if}
