<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	import { onDestroy } from 'svelte';
	import ModelSelector from './ModelSelector.svelte';

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

	type Props = {
		onConversationStart?: (id: string) => void;
	};

	let { onConversationStart }: Props = $props();

	let conversationId = $state<string | null>(null);
	let messages = $state<ChatMessage[]>([]);
	let inputValue = $state('');
	let isGenerating = $state(false);
	let isLoadingModel = $state(false);
	let error = $state<string | null>(null);

	// Streaming state: blocks being built for current response
	let streamingBlocks = $state<ContentBlock[]>([]);

	// Model state
	let modelSelector: ModelSelector;
	let modelReady = $state(false);
	let currentModelId = $state<string | null>(null);

	let unlistenAgent: UnlistenFn | undefined;
	let messagesContainer: HTMLElement | undefined;

	async function handleModelReady(modelId: string) {
		modelReady = true;
		currentModelId = modelId;
		await startChat();
	}

	async function startChat() {
		if (!currentModelId) return;
		try {
			isLoadingModel = true;
			error = null;
			const conv = await invoke<Conversation>('start_chat', {
				modelId: currentModelId,
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
			isLoadingModel = false;
		}
	}

	/** Load an existing conversation by ID */
	export async function loadConversation(id: string) {
		try {
			isLoadingModel = true;
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
			isLoadingModel = false;
		}
	}

	/** Start a new conversation (reset state) */
	export async function newConversation() {
		unlistenAgent?.();
		conversationId = null;
		messages = [];
		streamingBlocks = [];
		error = null;

		if (modelReady) {
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
		isGenerating = true;
		streamingBlocks = [];

		try {
			await invoke('send_message', {
				conversationId,
				message: userMessage,
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

	function handleAgentEvent(event: { payload: AgentEvent }) {
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

		// Auto-scroll
		if (messagesContainer) {
			setTimeout(() => {
				if (messagesContainer) {
					messagesContainer.scrollTop = messagesContainer.scrollHeight;
				}
			}, 0);
		}
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter' && !e.shiftKey) {
			e.preventDefault();
			sendMessage();
		}
	}

	onDestroy(() => {
		unlistenAgent?.();
	});
</script>

<div class="flex h-full flex-col">
	<!-- Messages Area -->
	<div
		bind:this={messagesContainer}
		class="flex-1 space-y-4 overflow-y-auto p-4"
	>
		{#if !modelReady}
			<ModelSelector
				bind:this={modelSelector}
				onModelReady={handleModelReady}
			/>
		{:else if isLoadingModel}
			<div class="flex h-full items-center justify-center">
				<div class="text-center text-slate-400">
					<div class="mb-2 text-lg">Loading model...</div>
					<div class="text-sm">This may take a moment</div>
				</div>
			</div>
		{:else if messages.length === 0}
			<div class="flex h-full items-center justify-center">
				<div class="text-center text-slate-400">
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
								? 'bg-rose-600 text-white'
								: 'bg-slate-700 text-slate-100'}"
						>
							<p class="whitespace-pre-wrap">{block.text}</p>
						</div>
					</div>
				{:else if block.type === 'tool_use'}
					<details class="mx-4 rounded border border-slate-600 bg-slate-800">
						<summary
							class="cursor-pointer px-2 py-1 text-xs text-slate-400 hover:text-slate-300"
						>
							Tool: {block.name}
						</summary>
						<div class="p-2">
							<pre
								class="max-h-24 overflow-auto text-xs text-slate-400">{JSON.stringify(
									block.arguments,
									null,
									2,
								)}</pre>
						</div>
					</details>
				{:else if block.type === 'tool_result'}
					<div
						class="mx-4 rounded border border-slate-600 bg-slate-800/50 p-2 text-xs {block.is_error
							? 'border-red-600/50'
							: ''}"
					>
						<pre
							class="max-h-48 overflow-auto text-slate-300 {block.is_error
								? 'text-red-300'
								: ''}">{block.content}</pre>
					</div>
				{/if}
			{/each}
		{/if}

		<!-- Streaming blocks -->
		{#if isGenerating}
			{#if streamingBlocks.length === 0}
				<div class="flex justify-start">
					<div
						class="max-w-[80%] rounded-lg bg-slate-700 px-4 py-2 text-slate-100"
					>
						<span class="animate-pulse text-slate-400">Generating...</span>
					</div>
				</div>
			{/if}

			{#each streamingBlocks as block, blockIdx (blockIdx)}
				{#if block.type === 'text'}
					<div class="flex justify-start">
						<div
							class="max-w-[80%] rounded-lg bg-slate-700 px-4 py-2 text-slate-100"
						>
							<p class="whitespace-pre-wrap">
								{block.text}<span class="animate-pulse">▊</span>
							</p>
						</div>
					</div>
				{:else if block.type === 'thinking'}
					<details
						class="mx-4 rounded border border-slate-600 bg-slate-800/50"
						open
					>
						<summary
							class="cursor-pointer px-2 py-1 text-xs italic text-slate-500 hover:text-slate-400"
						>
							Thinking...
						</summary>
						<p class="whitespace-pre-wrap p-2 text-xs text-slate-400">
							{block.thinking}
						</p>
					</details>
				{:else if block.type === 'tool_use'}
					<div
						class="mx-4 rounded border border-slate-600 bg-slate-800 p-2 text-xs"
					>
						<div class="flex items-center gap-2 font-medium text-slate-400">
							<span>Tool: {block.name}</span>
							{#if !block.result}
								<span class="animate-pulse">...</span>
							{:else if block.result.is_error}
								<span class="text-red-400">(error)</span>
							{:else}
								<span class="text-green-400">(done)</span>
							{/if}
						</div>
						{#if block.result}
							<pre
								class="mt-2 max-h-32 overflow-auto text-slate-300">{block.result.content.slice(
									0,
									300,
								)}{block.result.content.length > 300 ? '...' : ''}</pre>
						{/if}
					</div>
				{/if}
			{/each}
		{/if}
	</div>

	<!-- Error display -->
	{#if error}
		<div class="border-t border-red-700 bg-red-900/50 px-4 py-2 text-red-300">
			{error}
		</div>
	{/if}

	<!-- Input Area -->
	<div class="border-t border-slate-700 p-4">
		<div class="flex gap-2">
			<input
				type="text"
				bind:value={inputValue}
				onkeydown={handleKeydown}
				placeholder="Ask about your documents..."
				disabled={isGenerating || isLoadingModel || !modelReady}
				class="flex-1 rounded-md border border-slate-600 bg-slate-900 px-4 py-2
               text-slate-100 placeholder-slate-500 focus:border-rose-500
               focus:outline-none disabled:opacity-50"
			/>
			{#if isGenerating}
				<button
					onclick={cancelGeneration}
					class="rounded-md bg-slate-600 px-4 py-2 font-medium text-white hover:bg-slate-500"
				>
					Cancel
				</button>
			{:else}
				<button
					onclick={sendMessage}
					disabled={!inputValue.trim() || isLoadingModel || !modelReady}
					class="rounded-md bg-rose-600 px-4 py-2 font-medium text-white
                 hover:bg-rose-700 disabled:opacity-50"
				>
					Send
				</button>
			{/if}
		</div>
	</div>
</div>
