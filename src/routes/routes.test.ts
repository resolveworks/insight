import { describe, it, expect } from 'vitest';
import { load as rootLoad } from './+page';
import { load as collectionLoad } from './(app)/collections/[collectionId]/+page';

function redirectFrom(run: () => void) {
	try {
		run();
	} catch (e) {
		return e as { status: number; location: string };
	}
	throw new Error('expected a redirect');
}

describe('route redirects', () => {
	it('sends the root to the collections list', () => {
		const redirect = redirectFrom(() => rootLoad());
		expect(redirect.status).toBe(307);
		expect(redirect.location).toBe('/collections');
	});

	it('opens a collection on its research tab', () => {
		const redirect = redirectFrom(() =>
			collectionLoad({
				params: { collectionId: 'col1' },
				// The loader only reads params; the rest is SvelteKit's business.
			} as Parameters<typeof collectionLoad>[0]),
		);
		expect(redirect.status).toBe(307);
		expect(redirect.location).toBe('/collections/col1/research');
	});
});
