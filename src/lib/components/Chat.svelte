<script lang="ts">
	import { tick } from 'svelte';
	import { resolve } from '$app/paths';
	import Button from './Button.svelte';
	import ButtonLink from './ButtonLink.svelte';
	import Input from './Input.svelte';
	import Alert from './Alert.svelte';
	import EmptyState from './EmptyState.svelte';
	import ActivityStepView from './ActivityStep.svelte';
	import Message from './Message.svelte';
	import * as chat from '#lib/stores/conversations.svelte.js';
	import { documentCountLabel } from '#lib/api/collection.js';
	import { transcriptBlocks, transcriptTurns } from '#lib/chat/activity.js';
	import type { Collection } from '#lib/types/documents.js';

	type Props = {
		collection: Collection;
		providerReady: boolean;
	};

	let { collection, providerReady }: Props = $props();

	const collectionId = $derived(collection.id);
	const collectionName = $derived(collection.name);
	const documentCount = $derived(collection.document_count);
	const hasDocuments = $derived(documentCount > 0);
	const column = 'mx-auto w-full max-w-2xl px-6';

	// Reactive reads from the conversations store
	const transcript = $derived(chat.getItems());
	const blocks = $derived(transcriptBlocks(transcript));
	const turns = $derived(transcriptTurns(blocks));
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

<div class="flex h-full flex-col bg-neutral-50">
	<!-- Transcript -->
	<div bind:this={messagesContainer} class="flex-1 overflow-y-auto">
		<div class="{column} py-6">
			{#if transcript.length === 0}
				{#if !hasDocuments}
					<EmptyState
						title={`No documents in ${collectionName}`}
						description="Research answers need sources. Add documents to this collection before asking questions."
					>
						{#snippet action()}
							<ButtonLink
								href={resolve(`collections/${collectionId}/documents`)}
								variant="ghost"
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
				<div class="flex flex-col gap-8">
					<!-- One renderer over the transcript: a run committing only flips
					     provisional/executing flags — blocks stay in place. A question is
					     its own turn, so a reply's activity and text sit closer to each
					     other than to the question that prompted them. -->
					{#each turns as turn (turn.key)}
						<div class="flex flex-col gap-4">
							{#each turn.blocks as block (block.key)}
								{#if block.kind === 'activity'}
									<!-- Consecutive reasoning/tool steps read as one subordinate trail. -->
									<div class="flex flex-col gap-1">
										{#each block.steps as step (step.key)}
											<ActivityStepView {step} />
										{/each}
									</div>
								{:else if block.item.kind === 'user_text'}
									<Message from="user" text={block.item.text} />
								{:else if block.item.kind === 'assistant_text'}
									<Message
										from="assistant"
										text={block.item.text}
										streaming={block.item.streaming}
									/>
								{/if}
							{/each}
						</div>
					{/each}

					<!-- Placeholder for the gap between send and the first delta. -->
					{#if isGenerating && !transcript.some((item) => item.provisional)}
						<div class="animate-pulse text-sm text-neutral-500">
							Thinking...
						</div>
					{/if}
				</div>
			{/if}
		</div>
	</div>

	<!-- Error display -->
	{#if error}
		<div class="{column} pb-4">
			<Alert>{error}</Alert>
		</div>
	{/if}

	{#if providerReady}
		<!-- Composer -->
		<div class="{column} shrink-0 pb-6">
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
					<Button variant="outline" onclick={cancelGeneration}>Cancel</Button>
				{:else}
					<Button
						variant="solid"
						onclick={sendMessage}
						disabled={!hasDocuments || !inputValue.trim()}
					>
						Send
					</Button>
				{/if}
			</div>
		</div>
	{:else}
		<!-- The provider form lives in Settings; research only points at it. -->
		<div class="{column} flex shrink-0 items-center justify-between gap-4 pb-6">
			<p class="text-sm text-neutral-500">
				No language model configured. Research needs one.
			</p>
			<ButtonLink href={resolve('settings')} variant="solid">
				Open Settings
			</ButtonLink>
		</div>
	{/if}
</div>
