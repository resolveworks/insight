// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
import { invoke } from '@tauri-apps/api/core';
import type { LayoutLoad } from './$types';
import type { CurrentProvider, ProviderStatus } from '#lib/api/provider.js';

export const ssr = false;

/** Supply the app-global provider configuration; this load never gates. */
export const load: LayoutLoad = async ({ depends }) => {
	depends('app:provider');

	const [current, status] = await Promise.all([
		invoke<CurrentProvider | null>('get_current_provider'),
		invoke<ProviderStatus>('get_provider_status'),
	]);

	return { provider: { current, status } };
};
