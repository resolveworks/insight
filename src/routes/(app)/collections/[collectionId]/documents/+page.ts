import { listDeletedDocuments, listDocuments } from '#lib/api/document.js';
import { dataKeys } from '#lib/invalidation.js';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ params, depends }) => {
	depends(dataKeys.collection(params.collectionId));

	const [documents, deletedDocuments] = await Promise.all([
		listDocuments(params.collectionId),
		listDeletedDocuments(params.collectionId),
	]);

	return { documents, deletedDocuments };
};
