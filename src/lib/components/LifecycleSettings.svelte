<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { onMount } from 'svelte';
	import {
		getLanguageState,
		getOcrState,
	} from '$lib/stores/provider-state.svelte';

	interface LifecycleConfig {
		chat_coexist: boolean;
		embedding_coexist: boolean;
		ocr_coexist: boolean;
	}

	let config = $state<LifecycleConfig>({
		chat_coexist: false,
		embedding_coexist: false,
		ocr_coexist: false,
	});

	const languageState = getLanguageState();
	const ocrState = getOcrState();

	// Hide the chat checkbox when a remote provider is configured: remote
	// providers don't consume local memory so the flag is meaningless.
	let chatIsLocal = $derived(languageState.providerType === 'local');
	// Hide the OCR checkbox until an OCR model is configured — until then
	// there's nothing to coexist with.
	let ocrConfigured = $derived(ocrState.modelId !== null);

	async function load() {
		try {
			config = await invoke<LifecycleConfig>('get_lifecycle_config');
		} catch (e) {
			console.error('Failed to load lifecycle config:', e);
		}
	}

	async function save() {
		try {
			await invoke('set_lifecycle_config', { config });
		} catch (e) {
			console.error('Failed to save lifecycle config:', e);
		}
	}

	function toggle(key: 'chat_coexist' | 'embedding_coexist' | 'ocr_coexist') {
		config[key] = !config[key];
		save();
	}

	onMount(load);
</script>

<div class="space-y-3">
	{#if chatIsLocal}
		<label class="flex items-start gap-3 cursor-pointer">
			<input
				type="checkbox"
				class="mt-0.5 cursor-pointer"
				checked={config.chat_coexist}
				onchange={() => toggle('chat_coexist')}
			/>
			<span class="text-sm">
				<span class="block text-neutral-700">
					Keep chat model loaded alongside other models
				</span>
				<span class="block text-xs text-neutral-500 mt-0.5">
					Requires more VRAM. When off, switching between chat and embedding
					unloads one to free memory for the other.
				</span>
			</span>
		</label>
	{/if}

	<label class="flex items-start gap-3 cursor-pointer">
		<input
			type="checkbox"
			class="mt-0.5 cursor-pointer"
			checked={config.embedding_coexist}
			onchange={() => toggle('embedding_coexist')}
		/>
		<span class="text-sm">
			<span class="block text-neutral-700">
				Keep embedding model loaded alongside other models
			</span>
			<span class="block text-xs text-neutral-500 mt-0.5">
				Requires more VRAM. When off, indexing unloads the chat model (and vice
				versa).
			</span>
		</span>
	</label>

	{#if ocrConfigured}
		<label class="flex items-start gap-3 cursor-pointer">
			<input
				type="checkbox"
				class="mt-0.5 cursor-pointer"
				checked={config.ocr_coexist}
				onchange={() => toggle('ocr_coexist')}
			/>
			<span class="text-sm">
				<span class="block text-neutral-700">
					Keep OCR model loaded alongside other models
				</span>
				<span class="block text-xs text-neutral-500 mt-0.5">
					Requires more VRAM. OCR models are heavy (3B–9B parameters); leaving
					this off lets the chat model stay resident while ingestion runs.
				</span>
			</span>
		</label>
	{/if}
</div>
