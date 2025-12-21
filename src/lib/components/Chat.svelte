<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	import { onDestroy } from 'svelte';
	import { SvelteMap } from 'svelte/reactivity';
	import ModelSelector from './ModelSelector.svelte';

	interface ToolCall {
		id: string;
		name: string;
		arguments: object;
		result?: string;
		isError?: boolean;
		isLoading?: boolean;
	}

	interface ChatMessage {
		role: 'user' | 'assistant';
		content: string;
		toolCalls?: ToolCall[];
	}

	interface AgentEvent {
		type: 'TextDelta' | 'ToolCallStart' | 'ToolCallResult' | 'Done' | 'Error';
		data?: {
			content?: string;
			id?: string;
			name?: string;
			arguments?: object;
			is_error?: boolean;
			message?: string;
		};
	}

	interface Conversation {
		id: string;
		title: string;
		messages: {
			role: 'system' | 'user' | 'assistant' | 'tool';
			content: string;
			tool_call_id?: string;
			tool_calls?: {
				index: number;
				id: string;
				name: string;
				arguments: string;
			}[];
		}[];
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
	let streamingContent = $state('');
	let activeToolCalls = new SvelteMap<string, ToolCall>();
	let error = $state<string | null>(null);

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

			// Convert backend messages to ChatMessage format (skip system messages)
			messages = conv.messages
				.filter((m) => m.role === 'user' || m.role === 'assistant')
				.map((m) => ({
					role: m.role as 'user' | 'assistant',
					content: m.content,
					toolCalls: m.tool_calls?.map((tc) => ({
						id: tc.id,
						name: tc.name,
						arguments: JSON.parse(tc.arguments || '{}'),
					})),
				}));

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
		streamingContent = '';
		activeToolCalls = new SvelteMap();
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
		messages = [...messages, { role: 'user', content: userMessage }];
		isGenerating = true;
		streamingContent = '';
		activeToolCalls = new SvelteMap();

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
			case 'TextDelta':
				if (payload.data?.content) {
					streamingContent += payload.data.content;
				}
				break;

			case 'ToolCallStart':
				if (payload.data?.id && payload.data?.name) {
					activeToolCalls.set(payload.data.id, {
						id: payload.data.id,
						name: payload.data.name,
						arguments: payload.data.arguments || {},
						isLoading: true,
					});
					activeToolCalls = new SvelteMap(activeToolCalls);
				}
				break;

			case 'ToolCallResult':
				if (payload.data?.id) {
					const tc = activeToolCalls.get(payload.data.id);
					if (tc) {
						tc.result = payload.data.content;
						tc.isError = payload.data.is_error;
						tc.isLoading = false;
						activeToolCalls = new SvelteMap(activeToolCalls);
					}
				}
				break;

			case 'Done': {
				// Finalize assistant message with all tool calls
				const toolCallsArray = Array.from(activeToolCalls.values());
				messages = [
					...messages,
					{
						role: 'assistant',
						content: streamingContent,
						toolCalls: toolCallsArray.length > 0 ? toolCallsArray : undefined,
					},
				];
				streamingContent = '';
				activeToolCalls = new SvelteMap();
				isGenerating = false;
				break;
			}

			case 'Error':
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
						<p class="whitespace-pre-wrap">{message.content}</p>

						{#if message.toolCalls && message.toolCalls.length > 0}
							<div class="mt-2 space-y-2">
								{#each message.toolCalls as tc (tc.id)}
									<details class="rounded border border-slate-600 bg-slate-800">
										<summary
											class="cursor-pointer px-2 py-1 text-xs text-slate-400 hover:text-slate-300"
										>
											Tool: {tc.name}
											{#if tc.isError}
												<span class="text-red-400">(error)</span>
											{/if}
										</summary>
										{#if tc.result}
											<pre
												class="max-h-48 overflow-auto p-2 text-xs text-slate-300">{tc.result}</pre>
										{/if}
									</details>
								{/each}
							</div>
						{/if}
					</div>
				</div>
			{/each}
		{/if}

		<!-- Streaming message -->
		{#if isGenerating && (streamingContent || activeToolCalls.size > 0)}
			<div class="flex justify-start">
				<div
					class="max-w-[80%] rounded-lg bg-slate-700 px-4 py-2 text-slate-100"
				>
					{#if streamingContent}
						<p class="whitespace-pre-wrap">{streamingContent}</p>
					{/if}

					{#each [...activeToolCalls.values()] as tc (tc.id)}
						<div
							class="mt-2 rounded border border-slate-600 bg-slate-800 p-2 text-xs"
						>
							<div class="flex items-center gap-2 font-medium text-slate-400">
								<span>Tool: {tc.name}</span>
								{#if tc.isLoading}
									<span class="animate-pulse">...</span>
								{/if}
							</div>
							{#if tc.result && !tc.isLoading}
								<pre
									class="mt-1 max-h-32 overflow-auto text-slate-300">{tc.result.slice(
										0,
										300,
									)}{tc.result.length > 300 ? '...' : ''}</pre>
							{/if}
						</div>
					{/each}
				</div>
			</div>
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
