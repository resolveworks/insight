<script lang="ts">
	import { tick, untrack } from 'svelte';
	import Markdown from './Markdown.svelte';
	import ProviderSelector from './ProviderSelector.svelte';
	import Button from './Button.svelte';
	import GhostInput from './GhostInput.svelte';
	import ErrorAlert from './ErrorAlert.svelte';
	import { isProviderReady } from '$lib/stores/provider-state.svelte';
	import * as chat from '$lib/stores/conversations.svelte';
	import type { AssistantContentBlock, UserContentBlock } from '$lib/types/rig';

	// Provider state
	const providerReady = $derived(isProviderReady());

	// Reactive reads from the conversations store
	const activeId = $derived(chat.getActiveId());
	const messages = $derived(chat.getActiveMessages());
	const collections = $derived(chat.getActiveCollections());
	const live = $derived(chat.getLiveState());
	const isGenerating = $derived(chat.getIsGenerating());
	const isLoading = $derived(chat.getIsLoading());
	const error = $derived(chat.getError());

	const hasCollection = $derived(collections.length > 0);

	type EmptyState = 'no-provider' | 'pick-collection' | 'no-messages' | null;

	const emptyState = $derived.by<EmptyState>(() => {
		if (!providerReady) return 'no-provider';
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
		if (providerReady) {
			untrack(() => {
				chat.ensureInitialized();
			});
		}
	});

	// Auto-scroll to bottom whenever chat content changes. Reading message /
	// live-stream lengths + the live tail text is what tells $effect to re-run.
	$effect(() => {
		const tailChars = live.text.length;
		const toolChars = live.toolCalls.reduce(
			(n, c) => n + c.name.length + c.args.length,
			0,
		);
		const hasContent =
			messages.length > 0 ||
			live.toolResults.length > 0 ||
			tailChars > 0 ||
			toolChars > 0;

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

	/** Human-readable rendering of a rig reasoning content list. */
	function reasoningText(
		block: Extract<AssistantContentBlock, { type: 'reasoning' }>,
	): string {
		return block.content
			.map((part) => {
				if (part.type === 'text') return part.content.text;
				if (part.type === 'summary') return part.content;
				return '';
			})
			.filter(Boolean)
			.join('\n');
	}

	/** Flatten tool-result content to displayable text. */
	function toolResultText(
		block: Extract<UserContentBlock, { type: 'tool_result' }>,
	): string {
		return block.content
			.map((part) => {
				if (part.type === 'text') return part.text;
				if (part.type === 'json') return JSON.stringify(part.value, null, 2);
				return `[${part.type}]`;
			})
			.join('\n');
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
					Choose a provider to enable chat. Only the conversation is sent to the
					provider — documents never leave your machine.
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
				{#if message.role === 'user'}
					{#each message.content as block, j (j)}
						{#if block.type === 'text'}
							<div class="flex justify-end">
								<div
									class="max-w-[80%] min-w-0 break-words rounded-lg bg-primary-600 px-4 py-2 text-white"
								>
									<Markdown content={block.text} />
								</div>
							</div>
						{:else if block.type === 'tool_result'}
							<div
								class="mx-4 rounded border border-neutral-300 bg-surface-dim p-2 text-xs"
							>
								<div class="mb-1 font-medium text-neutral-500">
									Tool result: {block.name}
								</div>
								<pre
									class="max-h-48 overflow-auto whitespace-pre-wrap break-words text-neutral-700">{toolResultText(
										block,
									)}</pre>
							</div>
						{/if}
					{/each}
				{:else}
					{#each message.content as block, j (j)}
						{#if block.type === 'text'}
							<div class="flex justify-start">
								<div
									class="max-w-[80%] min-w-0 break-words rounded-lg border border-neutral-200 bg-surface-bright px-4 py-2 text-neutral-800"
								>
									<Markdown content={block.text} />
								</div>
							</div>
						{:else if block.type === 'reasoning'}
							<details
								class="mx-4 rounded border border-neutral-300 bg-surface-bright"
							>
								<summary
									class="cursor-pointer px-2 py-1 text-xs text-neutral-500 hover:text-neutral-700"
								>
									Reasoning
								</summary>
								<pre
									class="max-h-48 overflow-auto whitespace-pre-wrap break-words p-2 text-xs text-neutral-600">{reasoningText(
										block,
									)}</pre>
							</details>
						{:else if block.type === 'tool_call'}
							<details
								class="mx-4 rounded border border-neutral-300 bg-surface-bright"
							>
								<summary
									class="cursor-pointer px-2 py-1 text-xs text-neutral-500 hover:text-neutral-700"
								>
									Tool: {block.function.name}
								</summary>
								<div class="p-2">
									<pre
										class="max-h-24 overflow-auto whitespace-pre-wrap break-words text-xs text-neutral-600">{JSON.stringify(
											block.function.arguments,
											null,
											2,
										)}</pre>
								</div>
							</details>
						{/if}
					{/each}
				{/if}
			{/each}
		{/if}

		<!-- Live assistant turn -->
		{#if isGenerating}
			{#if live.reasoning.length > 0}
				<details
					class="mx-4 rounded border border-neutral-300 bg-surface-bright"
					open
				>
					<summary
						class="cursor-pointer px-2 py-1 text-xs text-neutral-500 hover:text-neutral-700"
					>
						Reasoning
					</summary>
					<pre
						class="max-h-48 overflow-auto whitespace-pre-wrap break-words p-2 text-xs text-neutral-600">{live.reasoning
							.map((r) => r.text)
							.join('\n')}</pre>
				</details>
			{/if}

			{#if live.text}
				<div class="flex justify-start">
					<div
						class="max-w-[80%] min-w-0 break-words rounded-lg border border-neutral-200 bg-surface-bright px-4 py-2 text-neutral-800"
					>
						<Markdown content={live.text} /><span
							class="animate-pulse text-primary-500">▊</span
						>
					</div>
				</div>
			{/if}

			{#each live.toolCalls as call (call.internal_call_id)}
				<div
					class="mx-4 rounded border border-neutral-300 bg-surface-bright p-2 text-xs"
				>
					<div class="flex items-center gap-2 font-medium text-neutral-500">
						<span>Tool: {call.name || '…'}</span>
						{#if !call.executed}
							<span class="animate-pulse text-primary-500">...</span>
						{/if}
					</div>
					{#if call.args}
						<pre
							class="mt-1 max-h-24 overflow-auto whitespace-pre-wrap break-words text-neutral-600">{call.args}</pre>
					{/if}
				</div>
			{/each}

			{#each live.toolResults as result (result.internal_call_id)}
				<div
					class="mx-4 rounded border border-neutral-300 bg-surface-dim p-2 text-xs"
				>
					<div class="mb-1 font-medium text-neutral-500">
						Tool result: {result.name}
					</div>
					<pre
						class="max-h-32 overflow-auto whitespace-pre-wrap break-words text-neutral-700">{result.text.slice(
							0,
							300,
						)}{result.text.length > 300 ? '...' : ''}</pre>
				</div>
			{/each}

			{#if live.toolCalls.length === 0 && live.toolResults.length === 0 && !live.text && live.reasoning.length === 0}
				<div class="flex justify-start">
					<div
						class="rounded-lg border border-neutral-200 bg-surface-bright px-4 py-2 text-neutral-500"
					>
						<span class="animate-pulse">Thinking...</span>
					</div>
				</div>
			{/if}
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
				disabled={isGenerating || !providerReady || !hasCollection}
			/>
			{#if isGenerating}
				<Button variant="secondary" onclick={cancelGeneration}>Cancel</Button>
			{:else}
				<Button
					onclick={sendMessage}
					disabled={!inputValue.trim() || !providerReady || !hasCollection}
				>
					Send
				</Button>
			{/if}
		</div>
	</div>
</div>
