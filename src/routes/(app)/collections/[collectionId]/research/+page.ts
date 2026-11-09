import { listConversations } from '#lib/api/conversation.js';
import { dataKeys } from '#lib/invalidation.js';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ params, depends }) => {
	depends(dataKeys.conversations(params.collectionId));

	return {
		conversations: await listConversations(params.collectionId),
	};
};
