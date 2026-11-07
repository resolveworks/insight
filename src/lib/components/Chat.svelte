<script lang="ts">
	import { tick } from 'svelte';
	import { resolve } from '$app/paths';
	import Markdown from './Markdown.svelte';
	import Button from './Button.svelte';
	import Input from './Input.svelte';
	import Alert from './Alert.svelte';
	import SetupScreen from './SetupScreen.svelte';
	import ToolBlock from './ToolBlock.svelte';
	import * as chat from '#lib/stores/conversations.svelte.js';
	import { documentCountLabel } from '#lib/api/collection.js';

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
	<!-- Messages Area -->
	<div
		bind:this={messagesContainer}
		class="flex-1 space-y-4 overflow-y-auto p-4"
	>
		{#if transcript.length === 0}
			{#if !hasDocuments}
				<div class="flex h-full items-center justify-center">
					<div class="max-w-md text-center">
						<div class="mb-2 text-lg text-neutral-800">
							No documents in {collectionName}
						</div>
						<p class="text-sm text-muted">
							Research answers need sources. Add documents to this collection
							before asking questions.
						</p>
						<a
							href={resolve(`collections/${collectionId}/documents`)}
							class="mt-4 inline-flex rounded-md border border-line-strong bg-surface-bright px-3 py-2 text-sm font-medium text-neutral-700 transition-colors hover:border-primary-300 hover:text-primary-600"
						>
							Go to Documents
						</a>
					</div>
				</div>
			{:else}
				<div class="flex h-full items-center justify-center">
					<div class="text-center text-muted">
						<div class="mb-2 text-lg text-neutral-800">
							Ask questions about {collectionName}
						</div>
						<div class="text-sm">
							{documentCountLabel(documentCount)} ready to search and read
						</div>
					</div>
				</div>
			{/if}
		{:else}
			<!-- One renderer over the transcript: a run committing only flips
			     provisional/executing flags — blocks stay in place. -->
			{#each transcript as item (item.key)}
				{#if item.kind === 'user_text'}
					<div class="flex justify-end">
						<div
							class="max-w-[80%] min-w-0 break-words rounded-lg bg-primary-600 px-4 py-2 text-white"
						>
							<Markdown content={item.text} />
						</div>
					</div>
				{:else if item.kind === 'assistant_text'}
					<div class="flex justify-start">
						<div
							class="max-w-[80%] min-w-0 break-words rounded-lg border border-line bg-surface-bright px-4 py-2 text-neutral-800"
						>
							<Markdown content={item.text} />{#if item.streaming}<span
									class="animate-pulse text-primary-500">▊</span
								>{/if}
						</div>
					</div>
				{:else if item.kind === 'reasoning'}
					<ToolBlock title="Reasoning">
						<pre
							class="max-h-48 overflow-auto whitespace-pre-wrap break-words text-xs text-neutral-600">{item.text}</pre>
					</ToolBlock>
				{:else if item.kind === 'tool_call'}
					<ToolBlock title="Tool: {item.name || '…'}">
						{#snippet summaryExtra()}
							{#if item.executing}
								<span class="animate-pulse text-primary-500">...</span>
							{/if}
						{/snippet}
						{#if item.text}
							<pre
								class="max-h-24 overflow-auto whitespace-pre-wrap break-words text-xs text-neutral-600">{item.text}</pre>
						{/if}
					</ToolBlock>
				{:else if item.kind === 'tool_result'}
					<ToolBlock
						title="Tool result: {item.name}"
						tone={item.is_error ? 'error' : 'neutral'}
					>
						<pre
							class="max-h-48 overflow-auto whitespace-pre-wrap break-words text-xs">{item.text}</pre>
					</ToolBlock>
				{/if}
			{/each}

			<!-- Placeholder for the gap between send and the first delta. -->
			{#if isGenerating && !transcript.some((item) => item.provisional)}
				<div class="flex justify-start">
					<div
						class="max-w-[80%] min-w-0 break-words rounded-lg border border-line bg-surface-bright px-4 py-2 text-muted"
					>
						<span class="animate-pulse">Thinking...</span>
					</div>
				</div>
			{/if}
		{/if}
	</div>

	<!-- Error display -->
	{#if error}
		<Alert>{error}</Alert>
	{/if}

	{#if providerReady}
		<!-- Input Area -->
		<div class="border-t border-line bg-surface-bright p-4">
			<div class="flex gap-2">
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
					<Button variant="secondary" onclick={cancelGeneration}>Cancel</Button>
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
	{:else}
		<!-- Provider setup lives below the transcript so local history stays readable -->
		<div class="h-80 max-h-[55%] shrink-0 overflow-y-auto border-t border-line">
			<SetupScreen />
		</div>
	{/if}
</div>
