<script lang="ts">
	import { onMount } from 'svelte';
	import Button from './Button.svelte';
	import Input from './Input.svelte';
	import Select from './Select.svelte';
	import Alert from './Alert.svelte';
	import {
		configureProvider,
		describeError,
		disableProvider,
		listModels,
		listProviders,
		type ModelChoice,
		type ProviderConfig,
	} from '#lib/api/provider.js';

	interface Props {
		provider: ProviderConfig | null;
		providerError: string | null;
	}

	let { provider, providerError }: Props = $props();

	const current = $derived(provider?.current);

	let selections = $state<string[]>([]);
	let family = $state('');
	let model = $state('');
	let apiKey = $state('');
	let baseUrl = $state('');
	let models = $state<ModelChoice[]>([]);
	let busy = $state(false);
	let error = $state<string | null>(null);

	const canSave = $derived(!busy && !!family && !!model.trim());
	const providerOptions = $derived(
		selections.map((id) => ({ value: id, label: id })),
	);
	const modelOptions = $derived(
		model && !models.some((m) => m.id === model)
			? [
					{ value: model, label: model },
					...models.map((m) => ({ value: m.id, label: m.name })),
				]
			: models.map((m) => ({ value: m.id, label: m.name })),
	);

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
			error = describeError(e);
		}
		const configured = current;
		if (configured) {
			family = configured.family;
			model = configured.model;
			apiKey = configured.api_key ?? '';
			baseUrl = configured.base_url ?? '';
			try {
				models = await listModels(configured.family);
			} catch (e) {
				error = describeError(e);
			}
		} else if (selections.length > 0) {
			await selectFamily(selections[0]);
		}
	});
</script>

<div class="flex flex-col gap-4">
	{#if providerError}
		<Alert>{providerError}</Alert>
	{:else if provider?.status.error}
		<Alert>{provider.status.error}</Alert>
	{/if}

	<Select
		label="Provider"
		bind:value={family}
		options={providerOptions}
		onchange={(e) => selectFamily(e.currentTarget.value)}
		disabled={busy}
		id="provider-family-select"
	/>

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

	<Select
		label="Model"
		bind:value={model}
		options={modelOptions}
		disabled={busy}
		id="model-input"
	/>

	<div class="flex gap-2">
		<Button variant="solid" onclick={save} disabled={!canSave} loading={busy}
			>Save</Button
		>
		{#if current}
			<Button variant="outline" onclick={disable} disabled={busy}
				>Disable</Button
			>
		{/if}
	</div>

	{#if error}
		<Alert>{error}</Alert>
	{/if}
</div>
