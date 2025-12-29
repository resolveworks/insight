import type { ModelType } from '$lib/stores/model-state.svelte';

/** Configuration for model selector component */
export interface ModelSelectorConfig {
	/** Model type for store lookup and unified commands */
	modelType: ModelType;
	/** Display title */
	title: string;
	/** Accent color */
	accentColor: 'slate' | 'emerald';
}

export const languageModelConfig: ModelSelectorConfig = {
	modelType: 'language',
	title: 'Language Model',
	accentColor: 'slate',
};

export const embeddingModelConfig: ModelSelectorConfig = {
	modelType: 'embedding',
	title: 'Embedding Model',
	accentColor: 'emerald',
};
