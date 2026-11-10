import { getDocument } from '#lib/api/document.js';
import { dataKeys } from '#lib/invalidation.js';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ params, depends }) => {
	depends(dataKeys.collection(params.collectionId));

	return {
		document: await getDocument(params.collectionId, params.documentId),
	};
};
