<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	import { onDestroy, onMount, tick } from 'svelte';
	import Markdown from './Markdown.svelte';
	import ProviderSelector from './ProviderSelector.svelte';
	import Button from './Button.svelte';
	import GhostInput from './GhostInput.svelte';
	import ErrorAlert from './ErrorAlert.svelte';

	// Content block types matching backend
	type ContentBlock =
		| { type: 'text'; text: string }
		| { type: 'tool_use'; id: string; name: string; arguments: object }
		| {
				type: 'tool_result';
				tool_use_id: string;
				content: string;
				is_error: boolean;
		  };

	// A chat message is a block with a role attached
	interface ChatMessage {
		role: 'user' | 'assistant';
		block: ContentBlock;
	}

	// Delta content for streaming
	type ContentDelta = { type: 'text'; text: string };

	// Agent events matching backend
	type AgentEvent =
		| { type: 'content_block_start'; data: { block: ContentBlock } }
		| { type: 'content_block_delta'; data: { delta: ContentDelta } }
		| { type: 'content_block_stop' }
		| { type: 'done' }
		| { type: 'error'; data: { message: string } };

	interface BackendMessage {
		role: 'system' | 'user' | 'assistant';
		content: ContentBlock[];
	}

	interface Conversation {
		id: string;
		title: string;
		messages: BackendMessage[];
		created_at: string;
		updated_at: string;
	}

	/** Collection info for filtering agent searches */
	interface CollectionInfo {
		id: string;
		name: string;
	}

	type Props = {
		onConversationStart?: (id: string) => void;
		/** Collections to filter agent searches to */
		collections?: CollectionInfo[];
		/** Initial conversation ID to load on mount */
		initialConversationId?: string | null;
	};

	let {
		onConversationStart,
		collections,
		initialConversationId = null,
	}: Props = $props();

	let conversationId = $state<string | null>(null);
	let messages = $state<ChatMessage[]>([]);
	let inputValue = $state('');
	let isGenerating = $state(false);
	let isLoading = $state(false);
	let error = $state<string | null>(null);

	// Streaming state: blocks being built for current response
	let streamingBlocks = $state<ContentBlock[]>([]);

	// Provider state
	let providerConfigured = $state(false);
	let checkingProvider = $state(true);

	// Prediction state (tab completion)
	let prediction = $state<string>('');
	let isPredicting = $state(false);
	let predictionTimeout: ReturnType<typeof setTimeout> | undefined;

	let unlistenAgent: UnlistenFn | undefined;
	let messagesContainer: HTMLElement | undefined;

	async function checkProviderStatus() {
		checkingProvider = true;
		try {
			const config = await invoke<object | null>('get_current_provider');
			providerConfigured = config !== null;
		} catch (e) {
			console.error('Failed to check provider status:', e);
			providerConfigured = false;
		} finally {
			checkingProvider = false;
		}
	}

	async function handleProviderConfigured() {
		providerConfigured = true;
		await startChat();
	}

	async function startChat() {
		if (!providerConfigured) return;
		try {
			isLoading = true;
			error = null;
			const conv = await invoke<Conversation>('start_chat', {
				collections: collections && collections.length > 0 ? collections : null,
			});
			conversationId = conv.id;
			messages = [];

			if (conversationId) {
				unlistenAgent = await listen<AgentEvent>(
					`agent-event-${conversationId}`,
					handleAgentEvent,
				);
				onConversationStart?.(conversationId);
			}
		} catch (e) {
			error = `Failed to start chat: ${e}`;
			console.error('Failed to start chat:', e);
		} finally {
			isLoading = false;
		}
	}

	/** Load an existing conversation by ID */
	export async function loadConversation(id: string) {
		try {
			isLoading = true;
			error = null;

			// Clean up existing listener
			unlistenAgent?.();

			const conv = await invoke<Conversation>('load_conversation', {
				conversationId: id,
			});
			conversationId = conv.id;

			// Flatten backend messages into individual blocks
			messages = conv.messages
				.filter((m) => m.role === 'user' || m.role === 'assistant')
				.flatMap((m) =>
					m.content.map((block) => ({
						role: m.role as 'user' | 'assistant',
						block,
					})),
				);

			// Set up event listener for this conversation
			unlistenAgent = await listen<AgentEvent>(
				`agent-event-${conversationId}`,
				handleAgentEvent,
			);
		} catch (e) {
			error = `Failed to load conversation: ${e}`;
			console.error('Failed to load conversation:', e);
		} finally {
			isLoading = false;

			// Scroll to the end of the conversation after DOM updates
			await tick();
			if (messagesContainer) {
				messagesContainer.scrollTop = messagesContainer.scrollHeight;
			}
		}
	}

	/** Start a new conversation (reset state) */
	export async function newConversation() {
		unlistenAgent?.();
		conversationId = null;
		messages = [];
		streamingBlocks = [];
		error = null;

		if (providerConfigured) {
			await startChat();
		}
	}

	async function sendMessage() {
		if (!inputValue.trim() || !conversationId || isGenerating) return;

		const userMessage = inputValue.trim();
		inputValue = '';
		error = null;

		// Add user message immediately
		messages = [
			...messages,
			{ role: 'user', block: { type: 'text', text: userMessage } },
		];

		// Scroll to show user message
		await tick();
		if (messagesContainer) {
			messagesContainer.scrollTop = messagesContainer.scrollHeight;
		}

		isGenerating = true;
		streamingBlocks = [];

		try {
			await invoke('send_message', {
				conversationId,
				message: userMessage,
				collections: collections && collections.length > 0 ? collections : null,
			});
		} catch (e) {
			error = `Failed to send message: ${e}`;
			console.error('Failed to send message:', e);
			isGenerating = false;
		}
	}

	async function cancelGeneration() {
		if (conversationId) {
			await invoke('cancel_generation', { conversationId });
			isGenerating = false;
		}
	}

	// Prediction functions (tab completion)
	async function requestPrediction() {
		if (!conversationId || isPredicting || isGenerating || inputValue) return;

		isPredicting = true;
		try {
			const result = await invoke<string | null>('predict_next_message', {
				conversationId,
			});
			// Only set prediction if input is still empty
			if (result && !inputValue) {
				prediction = result;
			}
		} catch (e) {
			console.error('Prediction failed:', e);
		} finally {
			isPredicting = false;
		}
	}

	async function cancelPrediction() {
		clearTimeout(predictionTimeout);
		if (conversationId) {
			try {
				await invoke('cancel_prediction', { conversationId });
			} catch {
				// Ignore cancellation errors
			}
		}
	}

	function handleAcceptPrediction() {
		prediction = '';
	}

	// Trigger prediction when input becomes empty
	$effect(() => {
		// Clear prediction if user types
		if (inputValue) {
			prediction = '';
			cancelPrediction();
			return;
		}

		// Don't predict during generation, loading, or without conversation
		if (isGenerating || isLoading || !conversationId || messages.length === 0) {
			prediction = '';
			return;
		}

		// Debounce prediction request (wait 500ms after input becomes empty)
		clearTimeout(predictionTimeout);
		predictionTimeout = setTimeout(() => {
			requestPrediction();
		}, 500);
	});

	async function handleAgentEvent(event: { payload: AgentEvent }) {
		const payload = event.payload;

		switch (payload.type) {
			case 'content_block_start':
				// Push new block
				streamingBlocks = [...streamingBlocks, payload.data.block];
				break;

			case 'content_block_delta': {
				// Update last block
				const lastIdx = streamingBlocks.length - 1;
				if (lastIdx >= 0) {
					const block = streamingBlocks[lastIdx];
					const delta = payload.data.delta;
					if (delta.type === 'text' && block.type === 'text') {
						streamingBlocks = [
							...streamingBlocks.slice(0, lastIdx),
							{ type: 'text', text: block.text + delta.text },
						];
					}
				}
				break;
			}

			case 'content_block_stop':
				// Block complete, nothing to do
				break;

			case 'done': {
				// Move streaming blocks to messages
				const newMessages = streamingBlocks.map((block) => ({
					role: 'assistant' as const,
					block,
				}));
				messages = [...messages, ...newMessages];
				streamingBlocks = [];
				isGenerating = false;
				break;
			}

			case 'error':
				error = payload.data?.message || 'Unknown error';
				console.error('Agent error:', payload.data?.message);
				isGenerating = false;
				break;
		}

		// Auto-scroll after DOM updates
		await tick();
		if (messagesContainer) {
			messagesContainer.scrollTop = messagesContainer.scrollHeight;
		}
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter' && !e.shiftKey) {
			e.preventDefault();
			sendMessage();
		}
	}

	onMount(async () => {
		await checkProviderStatus();
		if (providerConfigured) {
			if (initialConversationId) {
				// Restore previous conversation
				await loadConversation(initialConversationId);
			} else if (!conversationId) {
				await startChat();
			}
		}
	});

	onDestroy(() => {
		unlistenAgent?.();
		clearTimeout(predictionTimeout);
	});
</script>

<div class="flex h-full flex-col bg-surface">
	<!-- Messages Area -->
	<div
		bind:this={messagesContainer}
		class="flex-1 space-y-4 overflow-y-auto p-4"
	>
		{#if checkingProvider}
			<div class="flex h-full items-center justify-center">
				<div class="text-center text-neutral-500">
					<div class="text-lg">Checking provider...</div>
				</div>
			</div>
		{:else if !providerConfigured}
			<div class="mx-auto max-w-xl pt-8">
				<h2 class="mb-4 text-lg font-medium text-neutral-800">
					Configure a Language Model
				</h2>
				<p class="mb-6 text-sm text-neutral-500">
					Choose a provider to enable chat. You can run models locally or use
					OpenAI/Anthropic APIs.
				</p>
				<div class="rounded-lg border border-neutral-300 bg-surface-bright p-6">
					<ProviderSelector onConfigured={handleProviderConfigured} />
				</div>
			</div>
		{:else if isLoading}
			<div class="flex h-full items-center justify-center">
				<div class="text-center text-neutral-500">
					<div class="mb-2 text-lg">Starting chat...</div>
					<div class="text-sm">This may take a moment</div>
				</div>
			</div>
		{:else if messages.length === 0}
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
				{#if block.type === 'text'}
					<div
						class="flex {message.role === 'user'
							? 'justify-end'
							: 'justify-start'}"
					>
						<div
							class="max-w-[80%] rounded-lg px-4 py-2 {message.role === 'user'
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
								class="max-h-24 overflow-auto text-xs text-neutral-600">{JSON.stringify(
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
							class="max-h-48 overflow-auto text-neutral-700 {block.is_error
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
							class="max-w-[80%] rounded-lg border border-neutral-200 bg-surface-bright px-4 py-2 text-neutral-800"
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
							class="max-h-32 overflow-auto text-neutral-700 {block.is_error
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
				placeholder="Ask about your documents..."
				disabled={isGenerating || isLoading || !providerConfigured}
			/>
			{#if isGenerating}
				<Button variant="secondary" onclick={cancelGeneration}>Cancel</Button>
			{:else}
				<Button
					onclick={sendMessage}
					disabled={!inputValue.trim() || isLoading || !providerConfigured}
				>
					Send
				</Button>
			{/if}
		</div>
	</div>
</div>
