<script lang="ts">
	import { tick } from 'svelte';
	import { resolve } from '$app/paths';
	import Markdown from './Markdown.svelte';
	import Button from './Button.svelte';
	import GhostInput from './GhostInput.svelte';
	import ErrorAlert from './ErrorAlert.svelte';
	import SetupScreen from './SetupScreen.svelte';
	import * as chat from '#lib/stores/conversations.svelte.js';

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
	const activeId = $derived(chat.getActiveId());
	const messages = $derived(chat.getActiveMessages());
	const transcript = $derived(chat.getTranscript());
	const isGenerating = $derived(chat.getIsGenerating());
	const isLoading = $derived(chat.getIsLoading());
	const error = $derived(chat.getError());

	// View-local state
	let inputValue = $state('');
	let prediction = $state<string>('');
	let isPredicting = $state(false);
	let predictionTimeout: ReturnType<typeof setTimeout> | undefined;
	let messagesContainer: HTMLElement | undefined;

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
		if (
			isGenerating ||
			isLoading ||
			!activeId ||
			messages.length === 0 ||
			!hasDocuments ||
			!providerReady
		) {
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
		{#if messages.length === 0}
			{#if !hasDocuments}
				<div class="flex h-full items-center justify-center">
					<div class="max-w-md text-center">
						<div class="mb-2 text-lg text-neutral-800">
							No documents in {collectionName}
						</div>
						<p class="text-sm text-neutral-500">
							Research answers need sources. Add documents to this collection
							before asking questions.
						</p>
						<a
							href={resolve(`collections/${collectionId}/documents`)}
							class="mt-4 inline-flex rounded-md border border-neutral-300 bg-surface-bright px-3 py-2 text-sm font-medium text-neutral-700 transition-colors hover:border-primary-300 hover:text-primary-600"
						>
							Go to Documents
						</a>
					</div>
				</div>
			{:else}
				<div class="flex h-full items-center justify-center">
					<div class="text-center text-neutral-500">
						<div class="mb-2 text-lg text-neutral-800">
							Ask questions about {collectionName}
						</div>
						<div class="text-sm">
							{documentCount}
							{documentCount === 1 ? 'document' : 'documents'} ready to search and
							read
						</div>
					</div>
				</div>
			{/if}
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
							<Markdown content={item.text} />{#if item.streaming}<span
									class="animate-pulse text-primary-500">▊</span
								>{/if}
						</div>
					</div>
				{:else if item.kind === 'reasoning'}
					<!-- Always open — nothing ever auto-folds when a run commits. The
					     static attribute is set once, so manual folds stick. -->
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
							class="max-h-48 overflow-auto whitespace-pre-wrap break-words p-2 text-xs text-neutral-600">{item.text}</pre>
					</details>
				{:else if item.kind === 'toolCall'}
					<details
						class="mx-4 rounded border border-neutral-300 bg-surface-bright"
						open
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

	{#if providerReady}
		<!-- Input Area -->
		<div class="border-t border-neutral-300 bg-surface-bright p-4">
			<div class="flex gap-2">
				<GhostInput
					type="text"
					bind:value={inputValue}
					ghostText={prediction}
					onkeydown={handleKeydown}
					onAcceptGhost={handleAcceptPrediction}
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
		<div
			class="h-80 max-h-[55%] shrink-0 overflow-y-auto border-t border-neutral-300"
		>
			<SetupScreen />
		</div>
	{/if}
</div>
