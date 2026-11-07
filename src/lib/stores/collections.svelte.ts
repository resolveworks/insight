/**
 * Global store for collections and their documents.
 *
 * The Rust side owns the truth; this module only mirrors it for the UI.
 * Changes the UI initiates re-read what they touched; changes the backend
 * initiates (import pipeline stages, peer sync) arrive as
 * `collection-invalidated` signals carrying only the collection id, and the
 * store re-reads the affected collection. Signals are coalesced so a burst of
 * entry writes causes one refresh rather than one per write.
 */
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export interface Collection {
	id: string;
	name: string;
	document_count: number;
	total_pages: number | null;
	created_at?: string;
}

export interface Document {
	id: string;
	name: string;
	file_type: string;
	page_count: number | null;
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
	index: StageProgress;
}

interface CollectionInvalidated {
	collection_id: string;
}

// Module-level reactive state. Replaced wholesale, never mutated in place.
let collections = $state.raw<Collection[]>([]);
let documents = $state.raw<Record<string, Document[]>>({});
let pipelineProgress = $state.raw<Record<string, PipelineProgress>>({});
let documentErrors = $state.raw<Record<string, string | null>>({});
let loading = $state(false);
let loaded = $state(false);
let error = $state<string | null>(null);

function stageIsActive(stage: StageProgress): boolean {
	return stage.pending > 0 || stage.active > 0;
}

function pipelineIsActive(progress: PipelineProgress): boolean {
	return (
		stageIsActive(progress.store) ||
		stageIsActive(progress.extract) ||
		stageIsActive(progress.index)
	);
}

// =============================================================================
// Mirror refresh
// =============================================================================

async function loadCollections() {
	loading = true;
	error = null;
	try {
		collections = await invoke<Collection[]>('get_collections');
		loaded = true;
	} catch (e) {
		console.error('Failed to load collections:', e);
		error = e instanceof Error ? e.message : String(e);
	} finally {
		loading = false;
	}
}

async function loadDocuments(collectionId: string) {
	documentErrors = { ...documentErrors, [collectionId]: null };
	try {
		const docs = await invoke<Document[]>('get_documents', { collectionId });
		documents = { ...documents, [collectionId]: docs };
	} catch (e) {
		documentErrors = {
			...documentErrors,
			[collectionId]: e instanceof Error ? e.message : String(e),
		};
		throw e;
	}
}

// Per-collection refresh bookkeeping: a burst of invalidations collapses into
// one follow-up read, and reads never overlap, so an older snapshot cannot
// overwrite a newer one.
const refreshing: Record<string, boolean> = {};
const stale: Record<string, boolean> = {};

function invalidate(collectionId: string) {
	if (refreshing[collectionId]) {
		stale[collectionId] = true;
		return;
	}
	void refresh(collectionId);
}

async function refresh(collectionId: string) {
	refreshing[collectionId] = true;
	try {
		do {
			stale[collectionId] = false;
			await Promise.all([loadCollections(), loadDocuments(collectionId)]);
		} while (stale[collectionId]);
	} catch (e) {
		console.error(`Failed to refresh collection ${collectionId}:`, e);
	} finally {
		refreshing[collectionId] = false;
	}
}

async function loadPipelineProgress() {
	try {
		const active = await invoke<PipelineProgress[]>('get_pipeline_progress');
		for (const progress of active) applyProgress(progress);
	} catch (e) {
		console.error('Failed to load pipeline progress:', e);
	}
}

function applyProgress(progress: PipelineProgress) {
	const next = { ...pipelineProgress };
	if (pipelineIsActive(progress)) {
		next[progress.collection_id] = progress;
	} else {
		// Finished collections are not tracked: nothing left to display.
		delete next[progress.collection_id];
	}
	pipelineProgress = next;
}

async function setupEventListeners() {
	await listen<CollectionInvalidated>('collection-invalidated', (event) => {
		invalidate(event.payload.collection_id);
	});
	await listen<PipelineProgress>('pipeline-progress', (event) => {
		applyProgress(event.payload);
	});
}

// Initialize on module load. The module is the app's singleton sync layer.
if (typeof window !== 'undefined') {
	void setupEventListeners();
	void loadCollections();
	void loadPipelineProgress();
}

// =============================================================================
// Collection queries
// =============================================================================

export function getCollections(): Collection[] {
	return collections;
}

export function getCollection(id: string): Collection | undefined {
	return collections.find((c) => c.id === id);
}

export function getDocuments(collectionId: string): Document[] {
	return documents[collectionId] ?? [];
}

/** Whether the store has received this collection's document listing. */
export function hasLoadedDocuments(collectionId: string): boolean {
	return documents[collectionId] !== undefined;
}

/** The error from this collection's most recent document load, if it failed. */
export function getDocumentsError(collectionId: string): string | null {
	return documentErrors[collectionId] ?? null;
}

/**
 * Load a collection's documents on first view. The list page calls this when
 * the route param changes; subsequent updates arrive through invalidation.
 */
export function ensureDocuments(collectionId: string) {
	if (documents[collectionId] === undefined) invalidate(collectionId);
}

export function isLoading(): boolean {
	return loading;
}

/** Whether the collection listing has loaded successfully at least once. */
export function hasLoadedCollections(): boolean {
	return loaded;
}

export function getError(): string | null {
	return error;
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
	const previousDocuments = documents;
	collections = collections.filter((c) => c.id !== collectionId);
	if (documents[collectionId] !== undefined) {
		const next = { ...documents };
		delete next[collectionId];
		documents = next;
	}

	try {
		await invoke('delete_collection', { collectionId });
		return true;
	} catch (e) {
		console.error('Failed to delete collection:', e);
		collections = previousCollections;
		documents = previousDocuments;
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
// Document mutations and pipeline progress
// =============================================================================

/**
 * Start importing files into a collection.
 *
 * Progress and the resulting document listing arrive through events; nothing
 * is refreshed here.
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
		applyProgress(progress);
		return true;
	} catch (e) {
		console.error('Failed to start import:', e);
		return false;
	}
}

/** Delete a document, then refresh the collection's listing. */
export async function deleteDocument(
	collectionId: string,
	documentId: string,
): Promise<void> {
	await invoke('delete_document', { collectionId, documentId });
	invalidate(collectionId);
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
	return progress !== undefined && pipelineIsActive(progress);
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
