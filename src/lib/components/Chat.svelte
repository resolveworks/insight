<script lang="ts">
	import { tick } from 'svelte';
	import { resolve } from '$app/paths';
	import Markdown from './Markdown.svelte';
	import Button from './Button.svelte';
	import ButtonLink from './ButtonLink.svelte';
	import Input from './Input.svelte';
	import Alert from './Alert.svelte';
	import EmptyState from './EmptyState.svelte';
	import ActivityStepView from './ActivityStep.svelte';
	import * as chat from '#lib/stores/conversations.svelte.js';
	import { documentCountLabel } from '#lib/api/collection.js';
	import { transcriptBlocks } from '#lib/chat/activity.js';

	type Props = {
		collectionId: string;
		collectionName: string;
		documentCount: number;
		providerReady: boolean;
	};

	let { collectionId, collectionName, documentCount, providerReady }: Props =
		$props();

	const hasDocuments = $derived(documentCount > 0);

	// Reactive reads from the conversations store
	const transcript = $derived(chat.getItems());
	const blocks = $derived(transcriptBlocks(transcript));
	const isGenerating = $derived(chat.getIsGenerating());
	const error = $derived(chat.getError());

	// View-local state
	let inputValue = $state('');
	let messagesContainer: HTMLElement | undefined;

	// Auto-scroll to bottom whenever chat content changes. Reading the
	// transcript length plus the last item's text size is what tells $effect
	// to re-run — text, reasoning and tool-call args all grow `text`.
	$effect(() => {
		const last = transcript[transcript.length - 1];
		if (transcript.length === 0 || last.text.length === 0) {
			return;
		}

		tick().then(() => {
			if (messagesContainer) {
				messagesContainer.scrollTop = messagesContainer.scrollHeight;
			}
		});
	});

	async function sendMessage() {
		if (!inputValue.trim() || isGenerating || !hasDocuments || !providerReady)
			return;
		const text = inputValue;
		inputValue = '';
		await chat.sendMessage(text);
	}

	async function cancelGeneration() {
		await chat.cancelGeneration();
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter' && !e.shiftKey) {
			e.preventDefault();
			sendMessage();
		}
	}
</script>

<div class="flex h-full flex-col bg-surface">
	<!-- Transcript -->
	<div bind:this={messagesContainer} class="flex-1 overflow-y-auto">
		<div class="mx-auto w-full max-w-3xl px-6 py-6">
			{#if transcript.length === 0}
				{#if !hasDocuments}
					<EmptyState
						title={`No documents in ${collectionName}`}
						description="Research answers need sources. Add documents to this collection before asking questions."
					>
						{#snippet action()}
							<ButtonLink
								href={resolve(`collections/${collectionId}/documents`)}
								variant="secondary"
							>
								Go to Documents
							</ButtonLink>
						{/snippet}
					</EmptyState>
				{:else}
					<EmptyState
						title={`Ask questions about ${collectionName}`}
						description={`${documentCountLabel(documentCount)} ready to search and read`}
					/>
				{/if}
			{:else}
				<div class="space-y-5">
					<!-- One renderer over the transcript: a run committing only flips
					     provisional/executing flags — blocks stay in place. -->
					{#each blocks as block (block.key)}
						{#if block.kind === 'activity'}
							<!-- Consecutive reasoning/tool steps read as one subordinate trail. -->
							<div class="my-2 space-y-0.5 pl-4">
								{#each block.steps as step (step.key)}
									<ActivityStepView {step} />
								{/each}
							</div>
						{:else if block.item.kind === 'user_text'}
							<div class="flex justify-end">
								<div
									class="ml-auto max-w-prose min-w-0 break-words rounded-lg border border-primary-200 bg-primary-50 px-4 py-2.5 text-neutral-900"
								>
									<Markdown content={block.item.text} />
								</div>
							</div>
						{:else if block.item.kind === 'assistant_text'}
							<div class="max-w-prose text-base leading-7 text-neutral-800">
								<Markdown
									content={block.item.text}
								/>{#if block.item.streaming}<span
										class="ml-0.5 inline-block h-4 w-0.5 animate-pulse bg-primary-500 align-text-bottom"
									></span>{/if}
							</div>
						{/if}
					{/each}

					<!-- Placeholder for the gap between send and the first delta. -->
					{#if isGenerating && !transcript.some((item) => item.provisional)}
						<div class="animate-pulse text-sm text-muted">Thinking...</div>
					{/if}
				</div>
			{/if}
		</div>
	</div>

	<!-- Error display -->
	{#if error}
		<div class="mx-auto w-full max-w-3xl px-6 pb-2">
			<Alert>{error}</Alert>
		</div>
	{/if}

	{#if providerReady}
		<!-- Composer -->
		<div class="shrink-0 border-t border-line bg-surface-bright">
			<div class="mx-auto w-full max-w-3xl px-6 py-3">
				<div class="flex items-center gap-2">
					<Input
						type="text"
						bind:value={inputValue}
						class="flex-1"
						onkeydown={handleKeydown}
						placeholder={hasDocuments
							? 'Ask about your documents...'
							: 'Add documents to start asking questions'}
						disabled={isGenerating || !hasDocuments}
					/>
					{#if isGenerating}
						<Button variant="secondary" onclick={cancelGeneration}
							>Cancel</Button
						>
					{:else}
						<Button
							onclick={sendMessage}
							disabled={!hasDocuments || !inputValue.trim()}
						>
							Send
						</Button>
					{/if}
				</div>
			</div>
		</div>
	{:else}
		<!-- The provider form lives in Settings; research only points at it. -->
		<div class="shrink-0 border-t border-line bg-surface-bright">
			<div
				class="mx-auto flex w-full max-w-3xl items-center justify-between gap-4 px-6 py-3"
			>
				<p class="text-sm text-muted">
					No language model configured. Research needs one.
				</p>
				<ButtonLink href={resolve('settings')} variant="secondary">
					Open Settings
				</ButtonLink>
			</div>
		</div>
	{/if}
</div>
