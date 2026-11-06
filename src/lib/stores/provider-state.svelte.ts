/**
 * Chat provider state.
 *
 * Wraps the backend's provider commands: rig's provider selections, the
 * catalog models of the selected vendor, the configured provider, and why it
 * cannot be used. Chat is gated on `isProviderReady()`.
 */
import { invoke } from '@tauri-apps/api/core';

// =============================================================================
// Types (backend contract)
// =============================================================================

/** A chat model rig's catalog lists for a provider, as the model field offers it. */
export interface ModelChoice {
	id: string;
	name: string;
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
	/** Why the configured provider cannot be used, if it cannot. */
	error: string | null;
}

// =============================================================================
// State
// =============================================================================

/** rig's provider selections, canonically spelled. */
let selections = $state<string[]>([]);
let currentProvider = $state<CurrentProvider | null>(null);
let status = $state<ProviderStatus>({
	family: null,
	model: null,
	error: null,
});
/** Why the last provider state query failed, if it did. */
let providerStateError = $state<string | null>(null);
let loaded = $state(false);

// =============================================================================
// Internal helpers
// =============================================================================

/**
 * The message of an error thrown by a command.
 *
 * Command errors arrive as `{ code, message }` objects rather than `Error`
 * instances.
 */
export function describeError(error: unknown): string {
	if (error instanceof Error) return error.message;
	if (typeof error === 'object' && error !== null && 'message' in error) {
		return String((error as { message: unknown }).message);
	}
	return String(error);
}

/** Query provider state from the backend. */
async function queryProviderState(): Promise<void> {
	try {
		const [current, statusRes] = await Promise.all([
			invoke<CurrentProvider | null>('get_current_provider'),
			invoke<ProviderStatus>('get_provider_status'),
		]);
		currentProvider = current;
		status = statusRes;
		providerStateError = null;
	} catch (e) {
		providerStateError = `Failed to load provider settings: ${describeError(e)}`;
		throw e;
	}
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

/** Load (or reload) the provider selection list. */
export async function loadSelections(): Promise<string[]> {
	try {
		selections = await invoke<string[]>('list_providers');
	} catch (e) {
		console.error('Failed to list providers:', e);
	}
	return selections;
}

/** The chat models rig's catalog lists for `family`'s vendor. */
export async function loadModels(family: string): Promise<ModelChoice[]> {
	return invoke<ModelChoice[]>('list_provider_models', { family });
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
}

/** Disable the configured provider. */
export async function disableProvider(): Promise<void> {
	await invoke('disable_provider');
	await queryProviderState();
}

// =============================================================================
// Reactive getters
// =============================================================================

export function getSelections(): string[] {
	return selections;
}

export function getCurrentProvider(): CurrentProvider | null {
	return currentProvider;
}

export function getStatus(): ProviderStatus {
	return status;
}

/** Whether chat is enabled: a provider is configured and usable. */
export function isProviderReady(): boolean {
	return status.family !== null && status.error === null;
}

/** Why the provider state query failed, if it did (malformed/inaccessible settings). */
export function getProviderStateError(): string | null {
	return providerStateError;
}

/** Whether the initial state snapshot has been fetched. */
export function isLoaded(): boolean {
	return loaded;
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
