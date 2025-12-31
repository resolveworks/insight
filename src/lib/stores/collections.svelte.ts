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

/** Per-stage progress counts */
export interface StageProgress {
	pending: number;
	active: number;
	completed: number;
	failed: number;
}

/** Pipeline progress for a collection across all stages */
export interface PipelineProgress {
	collection_id: string;
	store: StageProgress;
	extract: StageProgress;
	embed: StageProgress;
	index: StageProgress;
}

interface DocumentAddedEvent {
	collection_id: string;
	document: Document;
}

// Module-level reactive state
let collections = $state<Collection[]>([]);
let loading = $state(false);
let error = $state<string | null>(null);
let pipelineProgress = $state<Record<string, PipelineProgress>>({});

// Track unlisten functions for cleanup
let unlistenDocAdded: UnlistenFn | null = null;
let unlistenPipelineProgress: UnlistenFn | null = null;

function updateCollectionDocCount(collectionId: string, delta: number) {
	collections = collections.map((c) =>
		c.id === collectionId
			? { ...c, document_count: c.document_count + delta }
			: c,
	);
}

/** Check if a stage has active work */
function stageIsActive(stage: StageProgress): boolean {
	return stage.pending > 0 || stage.active > 0;
}

/** Check if any stage in the pipeline is active */
function pipelineIsActive(progress: PipelineProgress): boolean {
	return (
		stageIsActive(progress.store) ||
		stageIsActive(progress.extract) ||
		stageIsActive(progress.embed) ||
		stageIsActive(progress.index)
	);
}

function updatePipelineProgress(progress: PipelineProgress) {
	if (!pipelineIsActive(progress)) {
		// Pipeline complete for this collection, remove from tracking
		const updated = { ...pipelineProgress };
		delete updated[progress.collection_id];
		pipelineProgress = updated;
	} else {
		pipelineProgress = {
			...pipelineProgress,
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

async function loadPipelineProgress() {
	try {
		const allProgress = await invoke<PipelineProgress[]>(
			'get_pipeline_progress',
		);
		for (const progress of allProgress) {
			updatePipelineProgress(progress);
		}
	} catch (e) {
		console.error('Failed to get pipeline progress:', e);
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

	unlistenPipelineProgress = await listen<PipelineProgress>(
		'pipeline-progress',
		(event) => {
			updatePipelineProgress(event.payload);
		},
	);
}

// Initialize on module load
if (typeof window !== 'undefined') {
	loadCollections();
	loadPipelineProgress();
	setupEventListeners();
}

/**
 * Cleanup event listeners. Call this on app unmount if needed.
 */
export function cleanup() {
	unlistenDocAdded?.();
	unlistenDocAdded = null;
	unlistenPipelineProgress?.();
	unlistenPipelineProgress = null;
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
// Document imports and pipeline progress
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
		const progress = await invoke<PipelineProgress>('start_import', {
			paths,
			collectionId,
		});
		updatePipelineProgress(progress);
		return true;
	} catch (e) {
		console.error('Failed to start import:', e);
		return false;
	}
}

/**
 * Get pipeline progress for a specific collection.
 */
export function getPipelineProgress(
	collectionId: string,
): PipelineProgress | undefined {
	return pipelineProgress[collectionId];
}

/**
 * Check if any pipeline stage is active for a collection.
 */
export function isProcessing(collectionId: string): boolean {
	const progress = pipelineProgress[collectionId];
	if (!progress) return false;
	return pipelineIsActive(progress);
}

/**
 * Get all active pipeline progress.
 */
export function getAllPipelineProgress(): PipelineProgress[] {
	return Object.values(pipelineProgress);
}

/**
 * Check if any pipeline is active globally.
 */
export function hasActivePipeline(): boolean {
	return Object.keys(pipelineProgress).length > 0;
}
