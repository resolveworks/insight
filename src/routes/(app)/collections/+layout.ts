import { listCollections } from '#lib/api/collection.js';
import { dataKeys } from '#lib/invalidation.js';
import type { LayoutLoad } from './$types';

export const load: LayoutLoad = async ({ depends }) => {
	depends(dataKeys.collections);
	return { collections: await listCollections() };
};
