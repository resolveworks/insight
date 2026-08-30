<script lang="ts">
	import { onMount } from 'svelte';
	import Button from './Button.svelte';
	import Input from './Input.svelte';
	import ErrorAlert from './ErrorAlert.svelte';
	import {
		configureProvider,
		disableProvider,
		getCurrentProvider,
		getFamilies,
		getFamily,
		getStoredApiKey,
		listProviderModels,
		loadFamilies,
		type ProviderFamily,
		type ProviderModel,
	} from '$lib/stores/provider-state.svelte';

	type Status = 'idle' | 'saving' | 'listing';

	let { onConfigured }: { onConfigured?: () => void } = $props();

	const families = $derived(getFamilies());
	const current = $derived(getCurrentProvider());

	let selectedFamilyId = $state<string>('');
	let apiKey = $state('');
	let baseUrl = $state('');
	let model = $state('');
	let models = $state<ProviderModel[] | null>(null);
	let listingFailed = $state(false);
	let status = $state<Status>('idle');
	let error = $state<string | null>(null);

	const family = $derived<ProviderFamily | undefined>(
		selectedFamilyId ? getFamily(selectedFamilyId) : undefined,
	);

	const isCurrent = $derived(
		current !== null &&
			current.family === selectedFamilyId &&
			current.model === model,
	);

	const canSave = $derived(
		status === 'idle' &&
			!!selectedFamilyId &&
			!!model.trim() &&
			(!family?.requires_api_key || !!apiKey.trim()),
	);

	function syncFromCurrent() {
		if (!current) return;
		selectedFamilyId = current.family;
		apiKey = current.api_key ?? getStoredApiKey(current.family) ?? '';
		baseUrl = current.base_url ?? '';
		model = current.model;
	}

	async function fetchModels(fam: ProviderFamily) {
		models = null;
		listingFailed = false;
		if (!fam.supports_model_listing) return;

		status = 'listing';
		try {
			const list = await listProviderModels({
				family: fam.id,
				apiKey: apiKey.trim() || null,
				baseUrl: baseUrl.trim() || null,
			});
			models = list;
			if (!model && list.length > 0) {
				model = list[0].id;
			}
		} catch (e) {
			// Listing failed (bad key, network, provider quirk): fall back to
			// manual model entry.
			console.error('Model listing failed:', e);
			models = null;
			listingFailed = true;
		} finally {
			status = 'idle';
		}
	}

	function selectFamily(id: string) {
		selectedFamilyId = id;
		error = null;
		models = null;
		listingFailed = false;
		model = '';
		apiKey = getStoredApiKey(id) ?? '';
		baseUrl = '';
		const fam = getFamily(id);
		if (!fam) return;

		if (current?.family === id) {
			apiKey = current.api_key ?? apiKey;
			baseUrl = current.base_url ?? '';
			model = current.model;
		}
		if (fam.supports_model_listing) {
			fetchModels(fam);
		}
	}

	async function save() {
		if (!canSave) return;
		status = 'saving';
		error = null;
		try {
			await configureProvider({
				family: selectedFamilyId,
				apiKey: family?.requires_api_key ? apiKey.trim() : null,
				baseUrl: family?.supports_base_url ? baseUrl.trim() || null : null,
				model: model.trim(),
			});
			onConfigured?.();
		} catch (e) {
			error = `Failed to configure provider: ${e}`;
		} finally {
			status = 'idle';
		}
	}

	async function handleDisable() {
		status = 'saving';
		error = null;
		try {
			await disableProvider();
			onConfigured?.();
		} catch (e) {
			error = `Failed to disable provider: ${e}`;
		} finally {
			status = 'idle';
		}
	}

	onMount(async () => {
		const list = await loadFamilies();
		if (current) {
			syncFromCurrent();
		} else if (list.length > 0) {
			selectedFamilyId = list[0].id;
			selectFamily(list[0].id);
		}
	});
</script>

<div class="space-y-4">
	<!-- Current provider indicator -->
	{#if current}
		<div
			class="mb-4 flex items-center gap-2 rounded-lg border border-primary-400 bg-primary-50 px-4 py-3 text-sm text-neutral-700"
		>
			<svg
				class="h-5 w-5 shrink-0 text-primary-500"
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
			<span data-testid="current-provider">
				{getFamily(current.family)?.name ?? current.family}: {current.model}
			</span>
			<button
				class="ml-auto cursor-pointer text-xs text-neutral-500 hover:text-neutral-700"
				onclick={handleDisable}
				disabled={status !== 'idle'}
			>
				Disable
			</button>
		</div>
	{/if}

	<!-- Family selection -->
	<div>
		<label
			for="provider-family-select"
			class="mb-2 block text-sm font-medium text-neutral-700"
		>
			Provider
		</label>
		<select
			id="provider-family-select"
			class="w-full cursor-pointer rounded-md border border-neutral-300 bg-surface-bright px-3 py-2 text-neutral-800 focus:border-transparent focus:ring-2 focus:ring-tertiary-400 focus:outline-none"
			bind:value={selectedFamilyId}
			onchange={(e) => selectFamily(e.currentTarget.value)}
			disabled={status !== 'idle'}
		>
			{#each families as fam (fam.id)}
				<option value={fam.id}>{fam.name}</option>
			{/each}
		</select>
	</div>

	<!-- API key (only when the family requires one) -->
	{#if family?.requires_api_key}
		<Input
			id="api-key-input"
			type="password"
			label="API Key"
			bind:value={apiKey}
			placeholder="API key"
			disabled={status !== 'idle'}
		/>
	{/if}

	<!-- Base URL (only when the family supports one) -->
	{#if family?.supports_base_url}
		<Input
			id="base-url-input"
			type="text"
			label="Base URL"
			bind:value={baseUrl}
			placeholder="https://api.example.com/v1"
			disabled={status !== 'idle'}
		/>
	{/if}

	<!-- Model: fetched dropdown when listing is supported, free text otherwise -->
	<div>
		<label
			for="model-input"
			class="mb-2 block text-sm font-medium text-neutral-700"
		>
			Model
		</label>
		{#if family?.supports_model_listing && models && models.length > 0}
			<select
				id="model-input"
				class="w-full cursor-pointer rounded-md border border-neutral-300 bg-surface-bright px-3 py-2 text-neutral-800 focus:border-transparent focus:ring-2 focus:ring-tertiary-400 focus:outline-none"
				bind:value={model}
				disabled={status !== 'idle'}
			>
				{#if !models.some((m) => m.id === model)}
					<option value={model}>{model}</option>
				{/if}
				{#each models as m (m.id)}
					<option value={m.id}>{m.display_name}</option>
				{/each}
			</select>
		{:else}
			<input
				id="model-input"
				type="text"
				class="w-full rounded-md border border-neutral-300 bg-surface-bright px-3 py-2 text-neutral-800 focus:border-transparent focus:ring-2 focus:ring-tertiary-400 focus:outline-none"
				bind:value={model}
				placeholder="Model id"
				disabled={status !== 'idle'}
			/>
			{#if listingFailed}
				<p class="mt-1 text-xs text-neutral-500">
					Could not load the model list — enter the model id manually.
				</p>
			{/if}
		{/if}
	</div>

	<Button
		fullWidth
		onclick={save}
		disabled={!canSave}
		loading={status === 'saving'}
	>
		{#if isCurrent}
			Active
		{:else}
			Save &amp; Activate
		{/if}
	</Button>

	{#if error}
		<ErrorAlert>{error}</ErrorAlert>
	{/if}
</div>
