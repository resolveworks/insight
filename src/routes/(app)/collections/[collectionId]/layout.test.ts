import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
import { createRawSnippet } from 'svelte';
import { page } from '$app/state';
import Layout from './+layout.svelte';
import { load } from './+layout';

vi.mock('@tauri-apps/api/event', () => ({
	listen: vi.fn(async () => () => {}),
}));

const collection = {
	id: 'col1',
	name: 'Climate Research',
	document_count: 2,
	total_pages: 10,
};

const document = {
	id: 'doc1',
	name: 'report.pdf',
	file_type: 'application/pdf',
	page_count: 12,
	tags: [],
	created_at: '2024-01-15T10:30:00Z',
};

const providerStatus = { family: null, model: null, error: null };

beforeEach(() => {
	mockIPC((cmd) => {
		if (cmd === 'get_documents') return [];
	});
});

function renderWorkspace(pathname: string) {
	page.url = new URL(`http://localhost${pathname}`);
	page.route.id = pathname.endsWith('/documents')
		? '/(app)/collections/[collectionId]/documents'
		: pathname.endsWith('/settings')
			? '/(app)/collections/[collectionId]/settings'
			: '/(app)/collections/[collectionId]/research';
	page.params = { collectionId: 'col1' };
	render(Layout, {
		props: {
			params: { collectionId: 'col1' },
			data: {
				collections: [collection],
				collection,
				documents: [],
				provider: { current: null, status: providerStatus },
				providerError: null,
			},
			children: createRawSnippet(() => ({ render: () => '<span></span>' })),
		},
	});
}

describe('Collection workspace load', () => {
	it('registers its dependency, finds the collection, and returns its documents', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_documents') return [document];
		});
		const depends = vi.fn();
		const parent = vi.fn(async () => ({ collections: [collection] }));

		const data = (await load({
			params: { collectionId: 'col1' },
			depends,
			parent,
		} as unknown as Parameters<typeof load>[0])) as {
			collection: unknown;
			documents: unknown[];
		};

		expect(depends).toHaveBeenCalledWith('app:collection:col1');
		expect(data.collection).toEqual(collection);
		expect(data.documents).toEqual([document]);
	});
});

describe('Collection workspace shell', () => {
	it('shows the collection name and every section tab', () => {
		renderWorkspace('/collections/col1/research');

		expect(
			screen.getByRole('heading', { name: 'Climate Research' }),
		).toBeInTheDocument();
		const links = screen.getAllByRole('link');
		expect(links.map((link) => link.textContent?.trim())).toEqual([
			'Research',
			'Documents',
			'Settings',
		]);
		expect(links.map((link) => link.getAttribute('href'))).toEqual([
			'/collections/col1/research',
			'/collections/col1/documents',
			'/collections/col1/settings',
		]);
		expect(links[0]).toHaveAttribute('aria-current', 'page');
		expect(links[1]).not.toHaveAttribute('aria-current');
		expect(links[2]).not.toHaveAttribute('aria-current');
	});

	it('marks Documents current on the document list', () => {
		renderWorkspace('/collections/col1/documents');

		const links = screen.getAllByRole('link');
		expect(links[0]).not.toHaveAttribute('aria-current');
		expect(links[1]).toHaveAttribute('aria-current', 'page');
	});

	it('marks Settings current on the settings page', () => {
		renderWorkspace('/collections/col1/settings');

		const links = screen.getAllByRole('link');
		expect(links[2]).toHaveAttribute('aria-current', 'page');
	});
});
