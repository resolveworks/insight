<script lang="ts">
	import { tick, untrack } from 'svelte';
	import CollectionFilter from './CollectionFilter.svelte';
	import Markdown from './Markdown.svelte';
	import Button from './Button.svelte';
	import GhostInput from './GhostInput.svelte';
	import ErrorAlert from './ErrorAlert.svelte';
	import { isProviderReady } from '$lib/stores/provider-state.svelte';
	import { resolve } from '$app/paths';
	import * as collectionsStore from '$lib/stores/collections.svelte';
	import * as chat from '$lib/stores/conversations.svelte';

	// Provider state
	const providerReady = $derived(isProviderReady());

	// Reactive reads from the conversations store
	const activeId = $derived(chat.getActiveId());
	const messages = $derived(chat.getActiveMessages());
	const collections = $derived(chat.getActiveCollections());
	const transcript = $derived(chat.getTranscript());
	const isGenerating = $derived(chat.getIsGenerating());
	const isLoading = $derived(chat.getIsLoading());
	const error = $derived(chat.getError());

	// Collections available to scope the conversation to
	const allCollections = $derived(collectionsStore.getCollections());

	const hasCollection = $derived(collections.length > 0);

	type EmptyState = 'pick-collection' | 'no-messages' | null;

	const emptyState = $derived.by<EmptyState>(() => {
		if (messages.length > 0) return null;
		return hasCollection ? 'no-messages' : 'pick-collection';
	});

	// View-local state
	let inputValue = $state('');
	let prediction = $state<string>('');
	let isPredicting = $state(false);
	let predictionTimeout: ReturnType<typeof setTimeout> | undefined;
	let messagesContainer: HTMLElement | undefined;

	// Conversations already auto-scoped this session. Deliberately
	// non-reactive and module-scoped: it survives remounts and only
	// dedupes, so a user removing scope is never re-added.
	// eslint-disable-next-line svelte/prefer-svelte-reactivity -- deliberately non-reactive
	const autoScoped = new Set<string>();
	let autoScopedDraft = false;
	let prevChatKey = 'init';

	// Initialize the store once the language provider is ready.
	$effect(() => {
		if (providerReady) {
			untrack(() => {
				chat.ensureInitialized();
			});
		}
	});

	// Single-collection auto-scope: a fresh, unscoped conversation (or draft)
	// gets the user's only collection added automatically. Fires at most once
	// per conversation id per session — and at most once per draft (the flag
	// resets whenever a new draft starts: after "New Chat" or deleting the
	// last conversation).
	$effect(() => {
		const chatKey = activeId ?? 'draft';
		if (
			activeId === null &&
			messages.length === 0 &&
			collections.length === 0 &&
			prevChatKey !== 'draft'
		) {
			autoScopedDraft = false;
		}
		// A conversation created from an auto-scoped draft inherits the flag,
		// so removing its scope later is never re-auto-scoped.
		if (prevChatKey === 'draft' && activeId !== null && autoScopedDraft) {
			autoScoped.add(activeId);
		}
		prevChatKey = chatKey;
	});

	$effect(() => {
		if (
			!providerReady ||
			messages.length > 0 ||
			collections.length > 0 ||
			allCollections.length !== 1
		) {
			return;
		}
		if (activeId) {
			if (autoScoped.has(activeId)) return;
			autoScoped.add(activeId);
		} else {
			if (autoScopedDraft) return;
			autoScopedDraft = true;
		}
		untrack(() => {
			void chat.addActiveCollection(allCollections[0]);
		});
	});

	// Auto-scroll to bottom whenever chat content changes. Reading the
	// transcript length plus the last item's text/args sizes is what tells
	// $effect to re-run — growing text AND growing tool-call args scroll.
	$effect(() => {
		const last = transcript[transcript.length - 1];
		if (
			transcript.length === 0 ||
			(last.text.length === 0 && last.args.length === 0)
		) {
			return;
		}

		tick().then(() => {
			if (messagesContainer) {
				messagesContainer.scrollTop = messagesContainer.scrollHeight;
			}
		});
	});

	async function handleAddCollection(id: string) {
		const found = collectionsStore.getCollection(id);
		if (found) await chat.addActiveCollection(found);
	}

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
		{#if emptyState === 'pick-collection'}
			{#if allCollections.length === 0}
				<div class="flex h-full items-center justify-center">
					<div class="w-full max-w-md text-center">
						<h2 class="mb-1 text-lg font-medium text-neutral-800">
							Choose collections to search
						</h2>
						<p class="mb-4 text-sm text-neutral-500">
							Collections hold the documents your agent searches.
						</p>
						<a
							href={resolve('/files')}
							class="inline-flex items-center justify-center rounded-md bg-primary-600 px-4 py-2 text-sm font-medium text-white hover:bg-primary-700"
						>
							Go to Files
						</a>
					</div>
				</div>
			{:else}
				<div class="flex h-full items-center justify-center">
					<div
						class="w-full max-w-md rounded-lg border border-neutral-200 bg-surface-bright p-6"
					>
						<h2 class="mb-1 text-lg font-medium text-neutral-800">
							Choose collections to search
						</h2>
						<p class="mb-4 text-sm text-neutral-500">
							Your agent searches only the collections you pick.
						</p>
						<CollectionFilter
							collections={allCollections}
							selected={collections}
							onAdd={handleAddCollection}
							onRemove={chat.removeActiveCollection}
						/>
					</div>
				</div>
			{/if}
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
			<!-- One renderer over the unified transcript: stored history and the
			     live run are the same view model, so a run committing only
			     flips provisional/executing flags — blocks stay in place. -->
			{#each transcript as item (item.key)}
				{#if item.kind === 'userText'}
					<div class="flex justify-end">
						<div
							class="max-w-[80%] min-w-0 break-words rounded-lg bg-primary-600 px-4 py-2 text-white"
						>
							<Markdown content={item.text} />
						</div>
					</div>
				{:else if item.kind === 'assistantText'}
					<div class="flex justify-start">
						<div
							class="max-w-[80%] min-w-0 break-words rounded-lg border border-neutral-200 bg-surface-bright px-4 py-2 text-neutral-800"
						>
							<Markdown content={item.text} /><span
								class="animate-pulse text-primary-500">▊</span
							>
						</div>
					</div>
				{:else if item.kind === 'reasoning'}
					<!-- open={item.provisional}: live blocks are expanded while you
					     watch them execute and fold to their summary line when the
					     run commits — flags flipping, not a different code path. -->
					<details
						class="mx-4 rounded border border-neutral-300 bg-surface-bright"
						open={item.provisional}
					>
						<summary
							class="cursor-pointer px-2 py-1 text-xs text-neutral-500 hover:text-neutral-700"
						>
							Reasoning
						</summary>
						<pre
							class="max-h-48 overflow-auto whitespace-pre-wrap break-words p-2 text-xs text-neutral-600">{item.text}</pre>
					</details>
				{:else if item.kind === 'toolCall'}
					<details
						class="mx-4 rounded border border-neutral-300 bg-surface-bright"
						open={item.provisional}
					>
						<summary
							class="flex cursor-pointer items-center gap-2 px-2 py-1 text-xs font-medium text-neutral-500 hover:text-neutral-700"
						>
							<span>Tool: {item.name || '…'}</span>
							{#if item.executing}
								<span class="animate-pulse text-primary-500">...</span>
							{/if}
						</summary>
						{#if item.args}
							<pre
								class="max-h-24 overflow-auto whitespace-pre-wrap break-words p-2 text-xs text-neutral-600">{item.args}</pre>
						{/if}
					</details>
				{:else if item.kind === 'toolResult'}
					<div
						class="mx-4 rounded border border-neutral-300 bg-surface-dim p-2 text-xs"
					>
						<div class="mb-1 font-medium text-neutral-500">
							Tool result: {item.name}
						</div>
						<pre
							class="max-h-48 overflow-auto whitespace-pre-wrap break-words text-neutral-700">{item.text}</pre>
					</div>
				{/if}
			{/each}

			<!-- Placeholder for the gap between send and the first delta. -->
			{#if isGenerating && !transcript.some((item) => item.provisional)}
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
