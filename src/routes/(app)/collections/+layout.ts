import { listCollections } from '#lib/api/collection.js';
import type { LayoutLoad } from './$types';

export const load: LayoutLoad = async ({ depends }) => {
	depends('app:collections');
	return { collections: await listCollections() };
};
