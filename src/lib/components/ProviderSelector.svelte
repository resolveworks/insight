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
	type ModelState = 'loading' | 'locked' | 'manual' | 'error' | 'ready';

	const DEBOUNCE_MS = 400;
	const LISTING_FAILED = 'Could not load the model list.';

	const modelControlClasses =
		'w-full cursor-pointer rounded-md border border-neutral-300 bg-surface-bright px-3 py-2 text-neutral-800 focus:border-transparent focus:ring-2 focus:ring-tertiary-400 focus:outline-none disabled:cursor-not-allowed disabled:bg-neutral-100 disabled:text-neutral-500';

	let { onConfigured }: { onConfigured?: () => void } = $props();

	const families = $derived(getFamilies());
	const current = $derived(getCurrentProvider());

	let selectedFamilyId = $state<string>('');
	let apiKey = $state('');
	let baseUrl = $state('');
	let model = $state('');
	let models = $state<ProviderModel[] | null>(null);
	let listingError = $state<string | null>(null);
	// Manual model entry: escape hatch when listing fails, or the default for
	// families whose rig client cannot list models.
	let manualEntry = $state(false);
	let status = $state<Status>('idle');
	let error = $state<string | null>(null);

	const family = $derived<ProviderFamily | undefined>(
		selectedFamilyId ? getFamily(selectedFamilyId) : undefined,
	);

	// Credentials come before models: listing is an authenticated call, so
	// the model control stays locked until the family's requirements are met.
	const needsKey = $derived(!!family?.requires_api_key && !apiKey.trim());
	const readyToList = $derived(
		!!family?.supports_model_listing && !manualEntry && !needsKey,
	);

	// One state machine drives the whole model control.
	const modelState = $derived.by<ModelState>(() => {
		if (!family) return 'loading';
		if (needsKey) return 'locked';
		if (!family.supports_model_listing || manualEntry) return 'manual';
		if (listingError) return 'error';
		return models?.length ? 'ready' : 'loading';
	});
	const modelPlaceholder = $derived(
		needsKey ? 'Enter your API key' : 'Loading models…',
	);
	const modelOptions = $derived(modelState === 'ready' && models ? models : []);

	const isCurrent = $derived(
		current !== null &&
			current.family === selectedFamilyId &&
			current.model === model,
	);

	const canSave = $derived(
		status === 'idle' && !!selectedFamilyId && !!model.trim() && !needsKey,
	);

	function syncFromCurrent() {
		if (!current) return;
		selectedFamilyId = current.family;
		apiKey = current.api_key ?? getStoredApiKey(current.family) ?? '';
		baseUrl = current.base_url ?? '';
		model = current.model;
	}

	async function fetchModels() {
		if (!family) return;
		status = 'listing';
		listingError = null;
		try {
			const list = await listProviderModels({
				family: family.id,
				apiKey: apiKey.trim() || null,
				baseUrl: baseUrl.trim() || null,
			});
			if (list.length === 0) throw new Error('provider returned no models');
			models = list;
			if (!model) model = list[0].id;
		} catch (e) {
			// Bad key, network, provider quirk — offer retry or manual entry.
			console.error('Model listing failed:', e);
			models = null;
			listingError = LISTING_FAILED;
		} finally {
			status = 'idle';
		}
	}

	// Fetch models whenever the credentials become (or change to) something
	// that might work — debounced so typing a key doesn't spam the provider.
	// `lastFetched` is deliberately non-reactive: it only dedupes.
	let lastFetched = '';
	$effect(() => {
		if (!readyToList) return;
		const sig = `${selectedFamilyId}|${apiKey.trim()}|${baseUrl.trim()}`;
		if (sig === lastFetched) return;
		const timer = setTimeout(() => {
			lastFetched = sig;
			void fetchModels();
		}, DEBOUNCE_MS);
		return () => clearTimeout(timer);
	});

	function selectFamily(id: string) {
		selectedFamilyId = id;
		error = null;
		models = null;
		listingError = null;
		manualEntry = false;
		model = '';
		apiKey = getStoredApiKey(id) ?? '';
		baseUrl = '';
		if (current?.family === id) {
			apiKey = current.api_key ?? apiKey;
			baseUrl = current.base_url ?? '';
			model = current.model;
		}
		// The credential effect above re-lists models when ready.
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

	<!-- Model: credentials first — listing-capable families load a dropdown
	     once the key is in; others (or a failed load) use manual entry. -->
	<div>
		<label
			for="model-input"
			class="mb-2 block text-sm font-medium text-neutral-700"
		>
			Model
		</label>
		{#if modelState === 'ready'}
			<select
				id="model-input"
				class={modelControlClasses}
				bind:value={model}
				disabled={status !== 'idle'}
			>
				{#if model && !modelOptions.some((m) => m.id === model)}
					<option value={model}>{model}</option>
				{/if}
				{#each modelOptions as m (m.id)}
					<option value={m.id}>{m.display_name}</option>
				{/each}
			</select>
		{:else if modelState === 'error'}
			<p role="alert" class="text-sm text-neutral-700">
				{LISTING_FAILED}
				<button
					type="button"
					class="cursor-pointer font-medium text-primary-600 hover:text-primary-700"
					onclick={() => fetchModels()}
				>
					Try again
				</button>
				or
				<button
					type="button"
					class="cursor-pointer font-medium text-primary-600 hover:text-primary-700"
					onclick={() => (manualEntry = true)}
				>
					enter a model id manually
				</button>
			</p>
		{:else if modelState === 'manual'}
			<input
				id="model-input"
				type="text"
				class={modelControlClasses}
				bind:value={model}
				placeholder="Model id"
				disabled={status !== 'idle'}
			/>
		{:else}
			<!-- locked (waiting for credentials) or loading -->
			<select id="model-input" class={modelControlClasses} disabled>
				<option value="">{modelPlaceholder}</option>
			</select>
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
