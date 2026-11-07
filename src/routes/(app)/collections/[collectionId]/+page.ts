import { redirect } from '@sveltejs/kit';
import { resolve } from '$app/paths';
import type { PageLoad } from './$types';

export const load: PageLoad = ({ params }) => {
	redirect(307, resolve(`collections/${params.collectionId}/research`));
};
