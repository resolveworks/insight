/** Configuration for model selector component */
export interface ModelSelectorConfig {
	/** Display title (used in download progress) */
	title: string;
	/** Tauri command to list available models */
	listCommand: string;
	/** Tauri command to get model status */
	statusCommand: string;
	/** Tauri command to download a model */
	downloadCommand: string;
	/** Tauri command to get current model */
	currentCommand: string;
	/** Tauri command to configure/load a model */
	configureCommand: string;
	/** Event name for download progress */
	progressEvent: string;
	/** Event name for download complete */
	completeEvent: string;
	/** Accent color */
	accentColor: 'slate' | 'emerald';
}

export const languageModelConfig: ModelSelectorConfig = {
	title: 'Language Model',
	listCommand: 'get_available_language_models',
	statusCommand: 'get_language_model_status',
	downloadCommand: 'download_language_model',
	currentCommand: 'get_current_language_model',
	configureCommand: 'configure_language_model',
	progressEvent: 'language-model-download-progress',
	completeEvent: 'language-model-download-complete',
	accentColor: 'slate',
};

export const embeddingModelConfig: ModelSelectorConfig = {
	title: 'Embedding Model',
	listCommand: 'get_available_embedding_models',
	statusCommand: 'get_embedding_model_status',
	downloadCommand: 'download_embedding_model',
	currentCommand: 'get_current_embedding_model',
	configureCommand: 'configure_embedding_model',
	progressEvent: 'embedding-model-download-progress',
	completeEvent: 'embedding-model-download-complete',
	accentColor: 'emerald',
};
