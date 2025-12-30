import type { ProviderType } from '$lib/stores/provider-state.svelte';

/** Configuration for model selector component */
export interface ModelSelectorConfig {
	/** Provider type for store lookup and unified commands */
	providerType: ProviderType;
	/** Display title */
	title: string;
	/** Accent color */
	accentColor: 'slate' | 'emerald';
}

export const languageModelConfig: ModelSelectorConfig = {
	providerType: 'language',
	title: 'Language Model',
	accentColor: 'slate',
};

export const embeddingModelConfig: ModelSelectorConfig = {
	providerType: 'embedding',
	title: 'Embedding Model',
	accentColor: 'emerald',
};
