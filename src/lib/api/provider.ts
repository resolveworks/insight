/**
 * Chat provider commands and the configuration the UI gates on.
 *
 * The provider is app-global storage truth owned by the backend; the root
 * layout load reads it, and mutations invalidate that load.
 */
import { invoke } from '@tauri-apps/api/core';
import { refreshProvider } from '#lib/invalidation.js';

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

/** The provider configuration the root layout load supplies. */
export interface ProviderConfig {
	current: CurrentProvider | null;
	status: ProviderStatus;
}

// =============================================================================
// Commands
// =============================================================================

/** Every provider selection rig registers, canonically spelled. */
export function listProviders(): Promise<string[]> {
	return invoke<string[]>('list_providers');
}

/** The chat models rig's catalog lists for `family`'s vendor. */
export function listModels(family: string): Promise<ModelChoice[]> {
	return invoke<ModelChoice[]>('list_provider_models', { family });
}

/** Configure a provider, then refresh the root layout's provider config. */
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
	await refreshProvider();
}

/** Disable the configured provider, then refresh the root layout's provider config. */
export async function disableProvider(): Promise<void> {
	await invoke('disable_provider');
	await refreshProvider();
}

// =============================================================================
// Pure helpers
// =============================================================================

/** Whether chat is enabled: a provider is configured and usable. */
export function isProviderReady(provider: ProviderConfig | null): boolean {
	return (
		provider !== null &&
		provider.status.family !== null &&
		provider.status.error === null
	);
}

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
