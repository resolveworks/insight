import { listConversations } from '#lib/api/conversation.js';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ params, depends }) => {
	depends(`app:conversations:${params.collectionId}`);

	return {
		conversations: await listConversations(params.collectionId),
	};
};
