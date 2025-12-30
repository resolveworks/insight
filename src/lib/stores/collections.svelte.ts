/**
 * Global store for collections state.
 * Provides a unified API for all collection operations including imports.
 * Queries backend for initial state and listens for real-time updates.
 */
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export interface Collection {
	id: string;
	name: string;
	document_count: number;
	total_pages: number;
	created_at?: string;
}

export interface Document {
	id: string;
	name: string;
	file_type: string;
	page_count: number;
	tags: string[];
	created_at: string;
}

export interface ImportProgress {
	collection_id: string;
	total: number;
	completed: number;
	failed: number;
	pending: number;
	in_progress: number;
}

export interface ProcessingProgress {
	collection_id: string;
	total: number;
	completed: number;
	failed: number;
	pending: number;
	in_progress: number;
}

interface DocumentAddedEvent {
	collection_id: string;
	document: Document;
}

// Module-level reactive state
let collections = $state<Collection[]>([]);
let loading = $state(false);
let error = $state<string | null>(null);
let importProgress = $state<Record<string, ImportProgress>>({});
let processingProgress = $state<Record<string, ProcessingProgress>>({});

// Track unlisten functions for cleanup
let unlistenDocAdded: UnlistenFn | null = null;
let unlistenImportProgress: UnlistenFn | null = null;
let unlistenProcessingProgress: UnlistenFn | null = null;

function updateCollectionDocCount(collectionId: string, delta: number) {
	collections = collections.map((c) =>
		c.id === collectionId
			? { ...c, document_count: c.document_count + delta }
			: c,
	);
}

function updateImportProgress(progress: ImportProgress) {
	if (progress.pending === 0 && progress.in_progress === 0) {
		// Import complete, remove from tracking
		const updated = { ...importProgress };
		delete updated[progress.collection_id];
		importProgress = updated;
	} else {
		importProgress = {
			...importProgress,
			[progress.collection_id]: progress,
		};
	}
}

function updateProcessingProgress(progress: ProcessingProgress) {
	if (progress.pending === 0 && progress.in_progress === 0) {
		// Processing complete, remove from tracking
		const updated = { ...processingProgress };
		delete updated[progress.collection_id];
		processingProgress = updated;
	} else {
		processingProgress = {
			...processingProgress,
			[progress.collection_id]: progress,
		};
	}
}

async function loadCollections() {
	if (loading) return;
	loading = true;
	error = null;

	try {
		collections = await invoke<Collection[]>('get_collections');
	} catch (e) {
		console.error('Failed to load collections:', e);
		error = e instanceof Error ? e.message : 'Failed to load collections';
	} finally {
		loading = false;
	}
}

async function loadImportProgress() {
	try {
		const allProgress = await invoke<ImportProgress[]>('get_import_progress');
		for (const progress of allProgress) {
			updateImportProgress(progress);
		}
	} catch (e) {
		console.error('Failed to get import progress:', e);
	}
}

async function loadProcessingProgress() {
	try {
		const allProgress = await invoke<ProcessingProgress[]>(
			'get_processing_progress',
		);
		for (const progress of allProgress) {
			updateProcessingProgress(progress);
		}
	} catch (e) {
		console.error('Failed to get processing progress:', e);
	}
}

async function setupEventListeners() {
	unlistenDocAdded = await listen<DocumentAddedEvent>(
		'document-added',
		(event) => {
			const { collection_id } = event.payload;
			updateCollectionDocCount(collection_id, 1);
		},
	);

	unlistenImportProgress = await listen<ImportProgress>(
		'import-progress',
		(event) => {
			updateImportProgress(event.payload);
		},
	);

	unlistenProcessingProgress = await listen<{ collection_id: string }>(
		'processing-progress',
		async () => {
			// Event only contains collection_id, fetch full progress
			try {
				const allProgress = await invoke<ProcessingProgress[]>(
					'get_processing_progress',
				);
				// Clear existing progress and update with fresh data
				processingProgress = {};
				for (const progress of allProgress) {
					updateProcessingProgress(progress);
				}
			} catch (e) {
				console.error('Failed to refresh processing progress:', e);
			}
		},
	);
}

// Initialize on module load
if (typeof window !== 'undefined') {
	loadCollections();
	loadImportProgress();
	loadProcessingProgress();
	setupEventListeners();
}

/**
 * Cleanup event listeners. Call this on app unmount if needed.
 */
export function cleanup() {
	unlistenDocAdded?.();
	unlistenDocAdded = null;
	unlistenImportProgress?.();
	unlistenImportProgress = null;
	unlistenProcessingProgress?.();
	unlistenProcessingProgress = null;
}

// =============================================================================
// Collection queries
// =============================================================================

/**
 * Get all collections (reactive).
 */
export function getCollections(): Collection[] {
	return collections;
}

/**
 * Check if store is loading.
 */
export function isLoading(): boolean {
	return loading;
}

/**
 * Get any error that occurred during loading.
 */
export function getError(): string | null {
	return error;
}

/**
 * Find a collection by ID.
 */
export function getCollection(id: string): Collection | undefined {
	return collections.find((c) => c.id === id);
}

/**
 * Refresh collections from backend.
 */
export async function refresh(): Promise<void> {
	await loadCollections();
}

// =============================================================================
// Collection mutations
// =============================================================================

/**
 * Create a new collection.
 * Returns the created collection or null on failure.
 */
export async function createCollection(
	name: string,
): Promise<Collection | null> {
	if (!name.trim()) return null;

	try {
		const collection = await invoke<Collection>('create_collection', { name });
		collections = [...collections, collection];
		return collection;
	} catch (e) {
		console.error('Failed to create collection:', e);
		return null;
	}
}

/**
 * Delete a collection.
 * Uses optimistic update - reverts on failure.
 * Returns true on success, false on failure.
 */
export async function deleteCollection(collectionId: string): Promise<boolean> {
	const previousCollections = collections;
	collections = collections.filter((c) => c.id !== collectionId);

	try {
		await invoke('delete_collection', { collectionId });
		return true;
	} catch (e) {
		console.error('Failed to delete collection:', e);
		collections = previousCollections;
		return false;
	}
}

/**
 * Generate a share ticket for a collection.
 * Returns the ticket string or null on failure.
 */
export async function shareCollection(
	collectionId: string,
): Promise<string | null> {
	try {
		return await invoke<string>('share_collection', {
			collectionId,
			writable: false,
		});
	} catch (e) {
		console.error('Failed to share collection:', e);
		return null;
	}
}

/**
 * Import a collection from a share ticket.
 * Returns the imported collection or null on failure.
 */
export async function importCollection(
	ticket: string,
): Promise<Collection | null> {
	try {
		const collection = await invoke<Collection>('import_collection', {
			ticket: ticket.trim(),
		});
		collections = [...collections, collection];
		return collection;
	} catch (e) {
		console.error('Failed to import collection:', e);
		return null;
	}
}

// =============================================================================
// Document imports
// =============================================================================

/**
 * Start importing files into a collection.
 * Progress updates are tracked automatically via events.
 */
export async function startImport(
	collectionId: string,
	paths: string[],
): Promise<boolean> {
	try {
		await invoke('start_import', { paths, collectionId });
		return true;
	} catch (e) {
		console.error('Failed to start import:', e);
		return false;
	}
}

/**
 * Get import progress for a specific collection.
 */
export function getImportProgress(
	collectionId: string,
): ImportProgress | undefined {
	return importProgress[collectionId];
}

/**
 * Check if any imports are active for a collection.
 */
export function isImporting(collectionId: string): boolean {
	const progress = importProgress[collectionId];
	if (!progress) return false;
	return progress.pending > 0 || progress.in_progress > 0;
}

/**
 * Get all active import progress.
 */
export function getAllImportProgress(): ImportProgress[] {
	return Object.values(importProgress);
}

/**
 * Check if any imports are active globally.
 */
export function hasActiveImports(): boolean {
	return Object.keys(importProgress).length > 0;
}

// =============================================================================
// Processing progress (embedding + indexing)
// =============================================================================

/**
 * Get processing progress for a specific collection.
 */
export function getProcessingProgress(
	collectionId: string,
): ProcessingProgress | undefined {
	return processingProgress[collectionId];
}

/**
 * Check if any processing is active for a collection.
 */
export function isProcessing(collectionId: string): boolean {
	const progress = processingProgress[collectionId];
	if (!progress) return false;
	return progress.pending > 0 || progress.in_progress > 0;
}

/**
 * Get all active processing progress.
 */
export function getAllProcessingProgress(): ProcessingProgress[] {
	return Object.values(processingProgress);
}

/**
 * Check if any processing is active globally.
 */
export function hasActiveProcessing(): boolean {
	return Object.keys(processingProgress).length > 0;
}
