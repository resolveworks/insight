import { error } from '@sveltejs/kit';
import { listDocuments } from '#lib/api/document.js';
import type { LayoutLoad } from './$types';

export const load: LayoutLoad = async ({ params, depends, parent }) => {
	depends(`app:collection:${params.collectionId}`);

	const { collections } = await parent();
	const collection = collections.find(
		(candidate) => candidate.id === params.collectionId,
	);
	if (!collection) error(404, 'Collection not found');

	return {
		collection,
		documents: await listDocuments(params.collectionId),
	};
};
