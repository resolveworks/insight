<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import Button from './Button.svelte';
	import Input from './Input.svelte';
	import Alert from './Alert.svelte';
	import {
		configureProvider,
		describeError,
		disableProvider,
		listModels,
		listProviders,
		type ModelChoice,
	} from '#lib/api/provider.js';

	const provider = $derived(page.data.provider);
	const current = $derived(provider.current);
	// Why the configured provider cannot be used (rig's validation error).
	const statusError = $derived(provider.status.error);

	let selections = $state<string[]>([]);
	let family = $state('');
	let model = $state('');
	let apiKey = $state('');
	let baseUrl = $state('');
	let models = $state<ModelChoice[]>([]);
	let busy = $state(false);
	let error = $state<string | null>(null);

	const canSave = $derived(!busy && !!family && !!model.trim());
	const selectClasses =
		'w-full cursor-pointer rounded-md border border-neutral-300 bg-surface-bright px-3 py-2 text-neutral-800 focus:border-transparent focus:ring-2 focus:ring-tertiary-400 focus:outline-none disabled:cursor-not-allowed disabled:bg-neutral-100 disabled:text-neutral-500';

	async function selectFamily(id: string) {
		family = id;
		model = '';
		error = null;
		let listed: ModelChoice[] = [];
		try {
			listed = await listModels(id);
		} catch (e) {
			error = describeError(e);
		}
		models = listed;
		model = listed[0]?.id ?? '';
	}

	async function save() {
		if (!canSave) return;
		busy = true;
		error = null;
		try {
			await configureProvider({
				family,
				apiKey: apiKey.trim() || null,
				baseUrl: baseUrl.trim() || null,
				model: model.trim(),
			});
		} catch (e) {
			error = describeError(e);
		} finally {
			busy = false;
		}
	}

	async function disable() {
		busy = true;
		error = null;
		try {
			await disableProvider();
		} catch (e) {
			error = describeError(e);
		} finally {
			busy = false;
		}
	}

	onMount(async () => {
		try {
			selections = await listProviders();
		} catch (e) {
			console.error('Failed to list providers:', e);
		}
		const configured = current;
		if (configured) {
			family = configured.family;
			model = configured.model;
			apiKey = configured.api_key ?? '';
			baseUrl = configured.base_url ?? '';
			models = await listModels(configured.family).catch(() => []);
		} else if (selections.length > 0) {
			await selectFamily(selections[0]);
		}
	});
</script>

<div class="space-y-4">
	{#if statusError}
		<Alert>{statusError}</Alert>
	{/if}

	<div>
		<label
			for="provider-family-select"
			class="mb-2 block text-sm font-medium text-neutral-700"
		>
			Provider
		</label>
		<select
			id="provider-family-select"
			class={selectClasses}
			bind:value={family}
			onchange={(e) => selectFamily(e.currentTarget.value)}
			disabled={busy}
		>
			{#each selections as id (id)}
				<option value={id}>{id}</option>
			{/each}
		</select>
	</div>

	<Input
		id="base-url-input"
		type="text"
		label="Base URL"
		bind:value={baseUrl}
		placeholder="https://api.example.com/v1"
		disabled={busy}
	/>

	<Input
		id="api-key-input"
		type="password"
		label="API Key"
		bind:value={apiKey}
		placeholder="API key"
		disabled={busy}
	/>

	<div>
		<label
			for="model-input"
			class="mb-2 block text-sm font-medium text-neutral-700"
		>
			Model
		</label>
		<select
			id="model-input"
			class={selectClasses}
			bind:value={model}
			disabled={busy}
		>
			{#if model && !models.some((m) => m.id === model)}
				<option value={model}>{model}</option>
			{/if}
			{#each models as m (m.id)}
				<option value={m.id}>{m.name}</option>
			{/each}
		</select>
	</div>

	<div class="flex gap-2">
		<Button fullWidth onclick={save} disabled={!canSave} loading={busy}>
			Save
		</Button>
		{#if current}
			<Button variant="secondary" onclick={disable} disabled={busy}
				>Disable</Button
			>
		{/if}
	</div>

	{#if error}
		<Alert>{error}</Alert>
	{/if}
</div>
