import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
import { page } from '$app/state';
import { invalidate } from '$app/navigation';
import ErrorPage from './+error.svelte';
import { load } from './+layout';

vi.mock('@tauri-apps/api/event', () => ({
	listen: vi.fn(async () => () => {}),
}));

describe('Collection workspace load for an unknown collection', () => {
	it('fails with 404 instead of rendering a generic collection title', async () => {
		mockIPC(() => []);

		let failure: unknown;
		try {
			await load({
				params: { collectionId: 'missing' },
				depends: vi.fn(),
				parent: async () => ({ collections: [] }),
			} as unknown as Parameters<typeof load>[0]);
		} catch (e) {
			failure = e;
		}

		expect(failure).toMatchObject({
			status: 404,
			body: { message: 'Collection not found' },
		});
	});

	it('shows a not-found state with a way back to collections', () => {
		page.url = new URL('http://localhost/collections/missing/research');
		page.params = { collectionId: 'missing' };
		page.status = 404;
		page.error = { message: 'Collection not found', status: 404 };

		render(ErrorPage);

		expect(screen.getByText('Collection not found')).toBeInTheDocument();
		expect(
			screen.getByRole('link', { name: 'Back to collections' }),
		).toHaveAttribute('href', '/collections');
	});

	it('shows a failed load and retries the collection', async () => {
		page.url = new URL('http://localhost/collections/missing/documents');
		page.params = { collectionId: 'missing' };
		page.status = 500;
		page.error = { message: 'storage unavailable', status: 500 };

		render(ErrorPage);

		expect(
			screen.getByText('Failed to load documents: storage unavailable'),
		).toBeInTheDocument();

		await fireEvent.click(screen.getByRole('button', { name: 'Retry' }));

		await waitFor(() =>
			expect(invalidate).toHaveBeenCalledWith('app:collection:missing'),
		);
	});
});
