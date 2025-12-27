<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { onMount } from 'svelte';
	import ModelDownloadSelector from './ModelDownloadSelector.svelte';
	import { languageModelConfig } from '$lib/models/config';

	interface ProviderFamily {
		id: string;
		name: string;
		description: string;
		requires_api_key: boolean;
	}

	interface RemoteModelInfo {
		id: string;
		name: string;
		description: string | null;
	}

	type ProviderConfig =
		| { type: 'local'; model_id: string }
		| { type: 'openai'; api_key: string; model: string }
		| { type: 'anthropic'; api_key: string; model: string };

	type Status = 'idle' | 'verifying' | 'configuring';

	type Props = {
		onConfigured?: () => void;
	};

	let { onConfigured }: Props = $props();

	let families = $state<ProviderFamily[]>([]);
	let selectedFamily = $state<string>('local');
	let currentProvider = $state<ProviderConfig | null>(null);

	// Remote provider state
	let apiKey = $state('');
	let models = $state<RemoteModelInfo[]>([]);
	let selectedModel = $state<string | null>(null);
	let status = $state<Status>('idle');
	let error = $state<string | null>(null);
	let isVerified = $state(false);

	// Check if the current provider matches selected family and model
	let isCurrentActive = $derived(() => {
		if (!currentProvider) return false;
		if (currentProvider.type !== selectedFamily) return false;
		if (currentProvider.type === 'local') return true;
		if (
			currentProvider.type === 'openai' ||
			currentProvider.type === 'anthropic'
		) {
			return currentProvider.model === selectedModel;
		}
		return false;
	});

	async function load() {
		try {
			families = await invoke<ProviderFamily[]>('get_provider_families');
			currentProvider = await invoke<ProviderConfig | null>(
				'get_current_provider',
			);

			// Set initial tab based on current provider
			if (currentProvider) {
				selectedFamily = currentProvider.type;

				// Pre-populate for remote providers
				if (
					currentProvider.type === 'openai' ||
					currentProvider.type === 'anthropic'
				) {
					apiKey = currentProvider.api_key;
					selectedModel = currentProvider.model;
					// Fetch models to populate dropdown
					await verifyApiKey();
				}
			}
		} catch (e) {
			error = `Failed to load providers: ${e}`;
		}
	}

	function selectFamily(id: string) {
		selectedFamily = id;
		// Reset remote state when switching
		error = null;
		isVerified = false;
		models = [];
		selectedModel = null;

		// Restore state if switching to current provider's family
		if (currentProvider?.type === id) {
			if (
				currentProvider.type === 'openai' ||
				currentProvider.type === 'anthropic'
			) {
				apiKey = currentProvider.api_key;
				selectedModel = currentProvider.model;
				// Re-verify to populate models
				verifyApiKey();
			}
		}
	}

	async function verifyApiKey() {
		if (!apiKey.trim()) {
			error = 'Please enter an API key';
			return;
		}

		status = 'verifying';
		error = null;

		try {
			const command =
				selectedFamily === 'openai'
					? 'fetch_openai_models'
					: 'fetch_anthropic_models';
			models = await invoke<RemoteModelInfo[]>(command, { apiKey });
			isVerified = true;
			if (models.length > 0 && !selectedModel) {
				selectedModel = models[0].id;
			}
		} catch (e) {
			error = `Verification failed: ${e}`;
			isVerified = false;
		} finally {
			status = 'idle';
		}
	}

	async function configureRemoteProvider() {
		if (!selectedModel || !apiKey) return;

		status = 'configuring';
		error = null;

		try {
			const command =
				selectedFamily === 'openai'
					? 'configure_openai_provider'
					: 'configure_anthropic_provider';
			await invoke(command, { apiKey, model: selectedModel });

			// Update current provider state
			currentProvider = {
				type: selectedFamily as 'openai' | 'anthropic',
				api_key: apiKey,
				model: selectedModel,
			};

			// Notify parent
			onConfigured?.();
		} catch (e) {
			error = `Failed to configure: ${e}`;
		} finally {
			status = 'idle';
		}
	}

	function handleLocalProviderConfigured(modelId: string | null) {
		if (modelId) {
			currentProvider = { type: 'local', model_id: modelId };
			onConfigured?.();
		}
	}

	async function disableProvider() {
		status = 'configuring';
		error = null;

		try {
			// Call configure with null to disable
			await invoke('configure_language_model', { modelId: null });
			currentProvider = null;
		} catch (e) {
			error = `Failed to disable: ${e}`;
		} finally {
			status = 'idle';
		}
	}

	onMount(load);
</script>

<div>
	<!-- Provider Tabs -->
	<div class="mb-6 flex border-b border-neutral-700">
		{#each families as family (family.id)}
			<button
				class="px-4 py-2 text-sm font-medium transition-colors -mb-px cursor-pointer
					{selectedFamily === family.id
					? 'text-amber-400 border-b-2 border-amber-400'
					: 'text-neutral-400 hover:text-neutral-200'}"
				onclick={() => selectFamily(family.id)}
			>
				{family.name}
			</button>
		{/each}
	</div>

	<!-- Current Provider Status (if active) -->
	{#if currentProvider && currentProvider.type === selectedFamily}
		<div
			class="flex items-center gap-2 px-4 py-3 rounded-lg border mb-4 text-sm border-amber-500 bg-amber-900/30 text-neutral-200"
		>
			<svg
				class="w-5 h-5 shrink-0 text-amber-500"
				viewBox="0 0 24 24"
				fill="none"
				stroke="currentColor"
			>
				<path
					stroke-linecap="round"
					stroke-linejoin="round"
					stroke-width="2"
					d="M5 13l4 4L19 7"
				/>
			</svg>
			<span>
				{#if currentProvider.type === 'local'}
					Local model active
				{:else}
					{currentProvider.type === 'openai' ? 'OpenAI' : 'Anthropic'}: {currentProvider.model}
				{/if}
			</span>
			<button
				class="ml-auto text-xs text-neutral-400 hover:text-neutral-200 cursor-pointer"
				onclick={disableProvider}
				disabled={status === 'configuring'}
			>
				Disable
			</button>
		</div>
	{/if}

	<!-- Provider Content -->
	{#if selectedFamily === 'local'}
		<ModelDownloadSelector
			config={languageModelConfig}
			onConfigured={handleLocalProviderConfigured}
		/>
	{:else}
		<!-- Remote Provider (OpenAI/Anthropic) -->
		<div class="space-y-4">
			<p class="text-sm text-neutral-400">
				{#if selectedFamily === 'openai'}
					Enter your OpenAI API key to access GPT models.
				{:else}
					Enter your Anthropic API key to access Claude models.
				{/if}
			</p>

			<!-- API Key Input -->
			<div>
				<label
					for="api-key-input"
					class="block text-sm font-medium text-neutral-300 mb-2"
				>
					API Key
				</label>
				<div class="flex gap-2">
					<input
						id="api-key-input"
						type="password"
						bind:value={apiKey}
						placeholder={selectedFamily === 'openai' ? 'sk-...' : 'sk-ant-...'}
						class="flex-1 px-3 py-2 bg-neutral-900 border border-neutral-600 rounded-md text-neutral-200 placeholder-neutral-500 focus:outline-none focus:ring-2 focus:ring-amber-500 focus:border-transparent"
						disabled={status !== 'idle'}
					/>
					<button
						class="px-4 py-2 rounded-md font-medium text-white cursor-pointer transition-colors bg-neutral-600 hover:bg-neutral-500 disabled:opacity-50 disabled:cursor-not-allowed"
						onclick={verifyApiKey}
						disabled={status !== 'idle' || !apiKey.trim()}
					>
						{#if status === 'verifying'}
							<svg
								class="w-4 h-4 inline-block animate-spin"
								viewBox="0 0 24 24"
								fill="none"
							>
								<circle
									class="opacity-25"
									cx="12"
									cy="12"
									r="10"
									stroke="currentColor"
									stroke-width="4"
								></circle>
								<path
									fill="currentColor"
									d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
								></path>
							</svg>
						{:else}
							Verify
						{/if}
					</button>
				</div>
			</div>

			<!-- Model Selection (shown after verification) -->
			{#if isVerified && models.length > 0}
				<div>
					<label
						for="model-select"
						class="block text-sm font-medium text-neutral-300 mb-2"
					>
						Model
					</label>
					<select
						id="model-select"
						bind:value={selectedModel}
						class="w-full px-3 py-2 bg-neutral-900 border border-neutral-600 rounded-md text-neutral-200 focus:outline-none focus:ring-2 focus:ring-amber-500 focus:border-transparent cursor-pointer"
						disabled={status !== 'idle'}
					>
						{#each models as model (model.id)}
							<option value={model.id}>
								{model.name}
								{#if model.description}
									- {model.description}
								{/if}
							</option>
						{/each}
					</select>
				</div>

				<!-- Activate Button -->
				<button
					class="w-full px-4 py-2 rounded-md font-medium text-white cursor-pointer transition-colors bg-amber-500 hover:bg-amber-600 disabled:opacity-50 disabled:cursor-not-allowed"
					onclick={configureRemoteProvider}
					disabled={status === 'configuring' ||
						!selectedModel ||
						isCurrentActive()}
				>
					{#if status === 'configuring'}
						<svg
							class="w-4 h-4 inline-block mr-2 animate-spin"
							viewBox="0 0 24 24"
							fill="none"
						>
							<circle
								class="opacity-25"
								cx="12"
								cy="12"
								r="10"
								stroke="currentColor"
								stroke-width="4"
							></circle>
							<path
								fill="currentColor"
								d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
							></path>
						</svg>
						Activating...
					{:else if isCurrentActive()}
						Active
					{:else}
						Activate
					{/if}
				</button>
			{/if}

			{#if error}
				<p class="text-sm text-red-400">{error}</p>
			{/if}
		</div>
	{/if}
</div>
