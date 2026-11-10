import { invoke } from '@tauri-apps/api/core';
import { refreshCollection } from '#lib/invalidation.js';
import type {
	DeletedDocument,
	Document,
	ImportOutcome,
	PipelineProgress,
} from '#lib/types/documents.js';

/** List the documents in a collection. */
export async function listDocuments(collectionId: string): Promise<Document[]> {
	return invoke<Document[]>('get_documents', { collectionId });
}

/** Read a document's metadata. */
export async function getDocument(
	collectionId: string,
	documentId: string,
): Promise<Document> {
	return invoke<Document>('get_document', { collectionId, documentId });
}

/** Read a document's extracted text. */
export async function getDocumentText(
	collectionId: string,
	documentId: string,
): Promise<string> {
	return invoke<string>('get_document_text', { collectionId, documentId });
}

/** Read a document's text chunks. */
export async function getDocumentChunks(
	collectionId: string,
	documentId: string,
): Promise<string[]> {
	return invoke<string[]>('get_document_chunks', {
		collectionId,
		documentId,
	});
}

/** List a collection's deleted documents. */
export async function listDeletedDocuments(
	collectionId: string,
): Promise<DeletedDocument[]> {
	return invoke<DeletedDocument[]>('get_deleted_documents', { collectionId });
}

/** Delete a document and re-read the collection and its document list. */
export async function deleteDocument(
	collectionId: string,
	documentId: string,
): Promise<void> {
	await invoke('delete_document', { collectionId, documentId });
	await refreshCollection(collectionId);
}

/** Restore a deleted document and re-read the collection and its document list. */
export async function undeleteDocument(
	collectionId: string,
	documentId: string,
): Promise<void> {
	await invoke('undelete_document', { collectionId, documentId });
	await refreshCollection(collectionId);
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
	await refreshCollection(collectionId);
	return outcome;
}

/** Read progress for every collection with active pipeline work. */
export async function getPipelineProgress(): Promise<PipelineProgress[]> {
	return invoke<PipelineProgress[]>('get_pipeline_progress');
}
