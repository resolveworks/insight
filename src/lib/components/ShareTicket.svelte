<script lang="ts">
	import Alert from './Alert.svelte';
	import Button from './Button.svelte';
	import LoadingSpinner from './LoadingSpinner.svelte';

	type State =
		| { status: 'loading' }
		| { status: 'ready'; ticket: string }
		| { status: 'error'; message: string };

	interface Props {
		state: State;
		copied: boolean;
		copyError: string | null;
		onCopy: () => void;
	}

	let { state, copied, copyError, onCopy }: Props = $props();
</script>

{#if state.status === 'ready'}
	<div class="flex flex-col gap-3">
		<div class="flex items-start gap-2">
			<code class="flex-1 font-mono text-xs break-all text-neutral-500">
				{state.ticket}
			</code>
			<Button size="sm" variant="outline" onclick={onCopy}>
				{copied ? 'Copied!' : 'Copy'}
			</Button>
		</div>
		<p class="text-xs text-neutral-400">
			Share this ticket with others so they can add, change, and delete
			documents in this collection.
		</p>
		{#if copyError}
			<Alert>{copyError}</Alert>
		{/if}
		<p class="text-xs text-neutral-500">
			Access cannot be revoked once shared. Syncing requires this peer to be
			available.
		</p>
	</div>
{:else if state.status === 'error'}
	<Alert>{state.message}</Alert>
{:else}
	<div class="flex items-center gap-2 text-sm text-neutral-400">
		<LoadingSpinner size="sm" />
		<span>Generating ticket...</span>
	</div>
{/if}
