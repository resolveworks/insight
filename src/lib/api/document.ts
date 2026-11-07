import { invoke } from '@tauri-apps/api/core';
import { invalidate } from '$app/navigation';
import type {
	Document,
	ImportOutcome,
	PipelineProgress,
} from '#lib/types/documents.js';

/** List the documents in a collection. */
export async function listDocuments(collectionId: string): Promise<Document[]> {
	return invoke<Document[]>('get_documents', { collectionId });
}

/** Delete a document and re-read the collection and its document list. */
export async function deleteDocument(
	collectionId: string,
	documentId: string,
): Promise<void> {
	await invoke('delete_document', { collectionId, documentId });
	await Promise.all([
		invalidate('app:collections'),
		invalidate(`app:collection:${collectionId}`),
	]);
}

/** Start importing files into a collection and re-read the affected data. */
export async function startImport(
	collectionId: string,
	paths: string[],
): Promise<ImportOutcome> {
	const outcome = await invoke<ImportOutcome>('start_import', {
		paths,
		collectionId,
	});
	await Promise.all([
		invalidate('app:collections'),
		invalidate(`app:collection:${collectionId}`),
	]);
	return outcome;
}

/** Read progress for every collection with active pipeline work. */
export async function getPipelineProgress(): Promise<PipelineProgress[]> {
	return invoke<PipelineProgress[]>('get_pipeline_progress');
}
