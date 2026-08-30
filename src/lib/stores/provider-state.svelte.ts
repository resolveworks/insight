/**
 * Chat provider state store.
 *
 * Wraps the backend's provider registry commands: the family list, the
 * currently configured provider, its runtime status, and stored API keys.
 * Chat is gated on `status.ready`.
 */
import { invoke } from '@tauri-apps/api/core';

// =============================================================================
// Types (backend contract)
// =============================================================================

export interface ProviderFamily {
	id: string;
	name: string;
	requires_api_key: boolean;
	supports_base_url: boolean;
	supports_model_listing: boolean;
}

export interface ProviderModel {
	id: string;
	display_name: string;
}

export interface CurrentProvider {
	family: string;
	model: string;
	api_key: string | null;
	base_url: string | null;
}

export interface ProviderStatus {
	family: string | null;
	model: string | null;
	ready: boolean;
}

// =============================================================================
// State
// =============================================================================

let families = $state<ProviderFamily[]>([]);
let currentProvider = $state<CurrentProvider | null>(null);
let status = $state<ProviderStatus>({
	family: null,
	model: null,
	ready: false,
});
let storedKeys = $state<Record<string, string>>({});
let loaded = $state(false);
let initializing = false;

// =============================================================================
// Internal helpers
// =============================================================================

/**
 * Query provider state from the backend. Safe to call repeatedly; call sites
 * that only need the current snapshot read the reactive getters instead.
 */
async function queryProviderState(): Promise<void> {
	const [current, statusRes, keys] = await Promise.all([
		invoke<CurrentProvider | null>('get_current_provider'),
		invoke<ProviderStatus>('get_provider_status'),
		invoke<Record<string, string>>('get_stored_api_keys'),
	]);
	currentProvider = current;
	status = statusRes;
	storedKeys = keys ?? {};
}

// Initialize on module load (browser only — the Tauri IPC bridge).
if (typeof window !== 'undefined') {
	queryProviderState()
		.catch((e) => {
			console.error('Failed to load provider state:', e);
		})
		.finally(() => {
			loaded = true;
		});
}

// =============================================================================
// Public API
// =============================================================================

/**
 * Resolve the initial provider state. Idempotent — safe to call from the app
 * layout boot flow. Resolves once the registry/status snapshot has loaded.
 */
export async function ensureProviderStateLoaded(): Promise<void> {
	if (loaded || initializing) return;
	initializing = true;
	try {
		if (!loaded) await queryProviderState().catch(() => {});
	} finally {
		loaded = true;
		initializing = false;
	}
}

/** Load (or reload) the provider family registry. */
export async function loadFamilies(): Promise<ProviderFamily[]> {
	try {
		families = await invoke<ProviderFamily[]>('list_provider_families');
	} catch (e) {
		console.error('Failed to list provider families:', e);
	}
	return families;
}

/**
 * Configure a provider. On success the store is refreshed from the backend
 * (the backend's status is authoritative).
 */
export async function configureProvider(opts: {
	family: string;
	apiKey?: string | null;
	baseUrl?: string | null;
	model: string;
}): Promise<void> {
	await invoke('configure_provider', {
		family: opts.family,
		apiKey: opts.apiKey ?? null,
		baseUrl: opts.baseUrl ?? null,
		model: opts.model,
	});
	await queryProviderState();
	// The key the backend stored is now available for re-selection.
	try {
		storedKeys = await invoke<Record<string, string>>('get_stored_api_keys');
	} catch {
		// Non-fatal: keys are a convenience for the settings UI.
	}
}

/** Disable the configured provider. */
export async function disableProvider(): Promise<void> {
	await invoke('disable_provider');
	await queryProviderState();
}

/**
 * Fetch the models a family offers. The caller supplies the candidate key /
 * base URL so listing can run before the provider is configured.
 */
export async function listProviderModels(opts: {
	family: string;
	apiKey?: string | null;
	baseUrl?: string | null;
}): Promise<ProviderModel[]> {
	return invoke<ProviderModel[]>('list_provider_models', {
		family: opts.family,
		apiKey: opts.apiKey ?? null,
		baseUrl: opts.baseUrl ?? null,
	});
}

// =============================================================================
// Reactive getters
// =============================================================================

export function getFamilies(): ProviderFamily[] {
	return families;
}

export function getCurrentProvider(): CurrentProvider | null {
	return currentProvider;
}

export function getStatus(): ProviderStatus {
	return status;
}

export function getStoredApiKeys(): Record<string, string> {
	return storedKeys;
}

export function getStoredApiKey(family: string): string | null {
	return storedKeys[family] ?? null;
}

/** Whether chat is enabled (a configured, ready provider exists). */
export function isProviderReady(): boolean {
	return status.ready;
}

/** Whether the initial state snapshot has been fetched. */
export function isLoaded(): boolean {
	return loaded;
}

export function getFamily(
	id: string | null | undefined,
): ProviderFamily | undefined {
	if (!id) return undefined;
	return families.find((f) => f.id === id);
}

/**
 * Test-only: re-query provider state from the (mocked) backend so a fresh
 * boot can be simulated without re-importing the module (which would
 * duplicate the Svelte runtime and break component rendering).
 */
export async function __resetForTests(): Promise<void> {
	loaded = false;
	await queryProviderState().catch(() => {});
	loaded = true;
}
