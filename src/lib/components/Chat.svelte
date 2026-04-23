<script lang="ts">
	import { tick, untrack } from 'svelte';
	import Markdown from './Markdown.svelte';
	import ProviderSelector from './ProviderSelector.svelte';
	import Button from './Button.svelte';
	import GhostInput from './GhostInput.svelte';
	import ErrorAlert from './ErrorAlert.svelte';
	import { getLanguageState } from '$lib/stores/provider-state.svelte';
	import * as chat from '$lib/stores/conversations.svelte';

	// Provider state
	const languageState = $derived(getLanguageState());
	const providerConfigured = $derived(languageState.ready);

	// Reactive reads from the conversations store
	const activeId = $derived(chat.getActiveId());
	const messages = $derived(chat.getActiveMessages());
	const collections = $derived(chat.getActiveCollections());
	const streamingBlocks = $derived(chat.getStreamingBlocks());
	const isGenerating = $derived(chat.getIsGenerating());
	const isLoading = $derived(chat.getIsLoading());
	const error = $derived(chat.getError());

	const hasCollection = $derived(collections.length > 0);

	type EmptyState = 'no-provider' | 'pick-collection' | 'no-messages' | null;

	const emptyState = $derived.by<EmptyState>(() => {
		if (!providerConfigured) return 'no-provider';
		if (messages.length > 0) return null;
		return hasCollection ? 'no-messages' : 'pick-collection';
	});

	// View-local state
	let inputValue = $state('');
	let prediction = $state<string>('');
	let isPredicting = $state(false);
	let predictionTimeout: ReturnType<typeof setTimeout> | undefined;
	let messagesContainer: HTMLElement | undefined;

	// Initialize the store once the language provider is ready.
	$effect(() => {
		if (providerConfigured) {
			untrack(() => {
				chat.ensureInitialized();
			});
		}
	});

	// Auto-scroll to bottom whenever chat content changes. Reading message /
	// streaming lengths + the live tail text is what tells $effect to re-run.
	$effect(() => {
		const lastStreaming = streamingBlocks.at(-1);
		const tailChars =
			lastStreaming?.type === 'text' ? lastStreaming.text.length : 0;
		const hasContent =
			messages.length > 0 || streamingBlocks.length > 0 || tailChars > 0;

		if (!hasContent) return;

		tick().then(() => {
			if (messagesContainer) {
				messagesContainer.scrollTop = messagesContainer.scrollHeight;
			}
		});
	});

	async function sendMessage() {
		if (!inputValue.trim() || isGenerating || !hasCollection) return;
		const text = inputValue;
		inputValue = '';
		await chat.sendMessage(text);
	}

	async function cancelGeneration() {
		await chat.cancelGeneration();
	}

	// Prediction (tab completion)
	async function requestPrediction() {
		if (!activeId || isPredicting || isGenerating || inputValue) return;
		isPredicting = true;
		try {
			const result = await chat.predictNextMessage();
			if (result && !inputValue) prediction = result;
		} finally {
			isPredicting = false;
		}
	}

	async function cancelPrediction() {
		clearTimeout(predictionTimeout);
		await chat.cancelPrediction();
	}

	function handleAcceptPrediction() {
		prediction = '';
	}

	$effect(() => {
		if (inputValue) {
			prediction = '';
			cancelPrediction();
			return;
		}
		if (isGenerating || isLoading || !activeId || messages.length === 0) {
			prediction = '';
			return;
		}
		clearTimeout(predictionTimeout);
		predictionTimeout = setTimeout(() => {
			requestPrediction();
		}, 500);
	});

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
		{#if emptyState === 'no-provider'}
			<div class="mx-auto max-w-xl pt-8">
				<h2 class="mb-4 text-lg font-medium text-neutral-800">
					Configure a Language Model
				</h2>
				<p class="mb-6 text-sm text-neutral-500">
					Choose a provider to enable chat. You can run models locally or use
					OpenAI/Anthropic APIs.
				</p>
				<div class="rounded-lg border border-neutral-300 bg-surface-bright p-6">
					<ProviderSelector />
				</div>
			</div>
		{:else if emptyState === 'pick-collection'}
			<div class="flex h-full items-center justify-center">
				<div class="text-center text-neutral-500">
					<div class="mb-2 text-lg">Pick a collection to begin</div>
					<div class="text-sm">
						Use the Collections filter on the right to scope your research.
					</div>
				</div>
			</div>
		{:else if emptyState === 'no-messages'}
			<div class="flex h-full items-center justify-center">
				<div class="text-center text-neutral-500">
					<div class="mb-2 text-lg">Ask questions about your documents</div>
					<div class="text-sm">
						The agent can search and read documents to help you find information
					</div>
				</div>
			</div>
		{:else}
			{#each messages as message, i (i)}
				{@const block = message.block}
				{#if message.role === 'context' && block.type === 'text'}
					<div class="flex justify-center">
						<div
							class="max-w-[80%] rounded-full bg-neutral-100 px-3 py-1 text-center text-xs text-neutral-500"
						>
							{block.text}
						</div>
					</div>
				{:else if block.type === 'text'}
					<div
						class="flex {message.role === 'user'
							? 'justify-end'
							: 'justify-start'}"
					>
						<div
							class="min-w-0 max-w-[80%] break-words rounded-lg px-4 py-2 {message.role ===
							'user'
								? 'bg-primary-600 text-white'
								: 'bg-surface-bright text-neutral-800 border border-neutral-200'}"
						>
							<Markdown content={block.text} />
						</div>
					</div>
				{:else if block.type === 'tool_use'}
					<details
						class="mx-4 rounded border border-neutral-300 bg-surface-bright"
					>
						<summary
							class="cursor-pointer px-2 py-1 text-xs text-neutral-500 hover:text-neutral-700"
						>
							Tool: {block.name}
						</summary>
						<div class="p-2">
							<pre
								class="max-h-24 overflow-auto whitespace-pre-wrap break-words text-xs text-neutral-600">{JSON.stringify(
									block.arguments,
									null,
									2,
								)}</pre>
						</div>
					</details>
				{:else if block.type === 'tool_result'}
					<div
						class="mx-4 rounded border border-neutral-300 bg-surface-dim p-2 text-xs {block.is_error
							? 'border-error/50'
							: ''}"
					>
						<pre
							class="max-h-48 overflow-auto whitespace-pre-wrap break-words text-neutral-700 {block.is_error
								? 'text-error'
								: ''}">{block.content}</pre>
					</div>
				{/if}
			{/each}
		{/if}

		<!-- Streaming blocks -->
		{#if isGenerating}
			{#each streamingBlocks as block, blockIdx (blockIdx)}
				{#if block.type === 'text'}
					<div class="flex justify-start">
						<div
							class="min-w-0 max-w-[80%] break-words rounded-lg border border-neutral-200 bg-surface-bright px-4 py-2 text-neutral-800"
						>
							<Markdown content={block.text} /><span
								class="animate-pulse text-primary-500">▊</span
							>
						</div>
					</div>
				{:else if block.type === 'tool_use'}
					<div
						class="mx-4 rounded border border-neutral-300 bg-surface-bright p-2 text-xs"
					>
						<div class="flex items-center gap-2 font-medium text-neutral-500">
							<span>Tool: {block.name}</span>
							<span class="animate-pulse text-primary-500">...</span>
						</div>
					</div>
				{:else if block.type === 'tool_result'}
					<div
						class="mx-4 rounded border border-neutral-300 bg-surface-dim p-2 text-xs {block.is_error
							? 'border-error/50'
							: ''}"
					>
						<pre
							class="max-h-32 overflow-auto whitespace-pre-wrap break-words text-neutral-700 {block.is_error
								? 'text-error'
								: ''}">{block.content.slice(0, 300)}{block.content.length > 300
								? '...'
								: ''}</pre>
					</div>
				{/if}
			{/each}
		{/if}
	</div>

	<!-- Error display -->
	{#if error}
		<ErrorAlert variant="banner">{error}</ErrorAlert>
	{/if}

	<!-- Input Area -->
	<div class="border-t border-neutral-300 bg-surface-bright p-4">
		<div class="flex gap-2">
			<GhostInput
				type="text"
				bind:value={inputValue}
				ghostText={prediction}
				onkeydown={handleKeydown}
				onAcceptGhost={handleAcceptPrediction}
				placeholder={hasCollection
					? 'Ask about your documents...'
					: 'Pick a collection to begin'}
				disabled={isGenerating ||
					isLoading ||
					!providerConfigured ||
					!hasCollection}
			/>
			{#if isGenerating}
				<Button variant="secondary" onclick={cancelGeneration}>Cancel</Button>
			{:else}
				<Button
					onclick={sendMessage}
					disabled={!inputValue.trim() ||
						isLoading ||
						!providerConfigured ||
						!hasCollection}
				>
					Send
				</Button>
			{/if}
		</div>
	</div>
</div>
