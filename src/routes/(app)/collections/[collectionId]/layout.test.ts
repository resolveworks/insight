import { describe, it, expect, beforeAll, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
import { createRawSnippet } from 'svelte';
import { page } from '$app/state';

vi.mock('@tauri-apps/api/event', () => ({
	listen: vi.fn(async () => () => {}),
}));

const collection = {
	id: 'col1',
	name: 'Climate Research',
	document_count: 2,
	total_pages: 10,
};

let Layout: typeof import('./+layout.svelte').default;

beforeAll(async () => {
	mockIPC((cmd) => {
		if (cmd === 'get_collections') return [collection];
		if (cmd === 'get_documents') return [];
		if (cmd === 'get_pipeline_progress') return [];
	});
	Layout = (await import('./+layout.svelte')).default;
});

function renderWorkspace(pathname: string) {
	page.url = new URL(`http://localhost${pathname}`);
	page.route.id = pathname.endsWith('/documents')
		? '/(app)/collections/[collectionId]/documents'
		: '/(app)/collections/[collectionId]/research';
	page.params = { collectionId: 'col1' };
	render(Layout, {
		props: {
			children: createRawSnippet(() => ({ render: () => '<span></span>' })),
		},
	});
}

describe('Collection workspace shell', () => {
	it('shows the collection name and both section tabs', async () => {
		renderWorkspace('/collections/col1/research');

		expect(
			await screen.findByRole('heading', { name: 'Climate Research' }),
		).toBeInTheDocument();
		const links = screen.getAllByRole('link');
		expect(links.map((link) => link.textContent?.trim())).toEqual([
			'Research',
			'Documents',
		]);
		expect(links.map((link) => link.getAttribute('href'))).toEqual([
			'/collections/col1/research',
			'/collections/col1/documents',
		]);
		expect(links[0]).toHaveAttribute('aria-current', 'page');
		expect(links[1]).not.toHaveAttribute('aria-current');
	});

	it('marks Documents current on the document list', async () => {
		renderWorkspace('/collections/col1/documents');

		await screen.findByRole('heading', { name: 'Climate Research' });
		const links = screen.getAllByRole('link');
		expect(links[0]).not.toHaveAttribute('aria-current');
		expect(links[1]).toHaveAttribute('aria-current', 'page');
	});
});
