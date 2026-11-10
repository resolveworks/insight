import { getCollectionSharingStatus } from '#lib/api/collection.js';
import { dataKeys } from '#lib/invalidation.js';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ params, depends }) => {
	depends(dataKeys.sharing);

	return {
		sharing: await getCollectionSharingStatus(params.collectionId),
	};
};
