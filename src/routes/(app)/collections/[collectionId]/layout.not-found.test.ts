import { describe, it, expect, beforeAll, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
import { createRawSnippet } from 'svelte';
import { page } from '$app/state';

vi.mock('@tauri-apps/api/event', () => ({
	listen: vi.fn(async () => () => {}),
}));

let Layout: typeof import('./+layout.svelte').default;

beforeAll(async () => {
	mockIPC((cmd) => {
		if (cmd === 'get_collections') return [];
		if (cmd === 'get_pipeline_progress') return [];
	});
	Layout = (await import('./+layout.svelte')).default;
});

describe('Collection workspace shell for an unknown collection', () => {
	it('shows a not-found state instead of a generic collection title', async () => {
		page.url = new URL('http://localhost/collections/missing/research');
		page.route.id = '/(app)/collections/[collectionId]/research';
		page.params = { collectionId: 'missing' };

		render(Layout, {
			props: {
				children: createRawSnippet(() => ({ render: () => '<span></span>' })),
			},
		});

		await waitFor(() => {
			expect(screen.getByText('Collection not found')).toBeInTheDocument();
		});
		expect(
			screen.getByRole('link', { name: 'Back to collections' }),
		).toHaveAttribute('href', '/collections');
		expect(screen.queryByText('Collection')).not.toBeInTheDocument();
	});
});
