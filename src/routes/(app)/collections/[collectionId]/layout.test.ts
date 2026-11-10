import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
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

const providerStatus = { family: null, model: null, error: null };

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
				provider: { current: null, status: providerStatus },
				providerError: null,
			},
			children: createRawSnippet(() => ({ render: () => '<span></span>' })),
		},
	});
}

describe('Collection workspace load', () => {
	it('registers its dependency and finds the collection without loading its documents', async () => {
		const depends = vi.fn();
		const parent = vi.fn(async () => ({ collections: [collection] }));

		const data = (await load({
			params: { collectionId: 'col1' },
			depends,
			parent,
		} as unknown as Parameters<typeof load>[0])) as { collection: unknown };

		expect(depends).toHaveBeenCalledWith('app:collection:col1');
		expect(data.collection).toEqual(collection);
		expect(data).not.toHaveProperty('documents');
	});
});

describe('Collection workspace shell', () => {
	it('shows the collection name and every section tab', () => {
		renderWorkspace('/collections/col1/research');

		expect(
			screen.getByRole('heading', { name: 'Climate Research' }),
		).toBeInTheDocument();
		expect(
			screen.getByRole('link', { name: /Back to collections/ }),
		).toHaveAttribute('href', '/collections');

		const links = screen.getAllByRole('link').slice(1);
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

		expect(screen.getByRole('link', { name: 'Research' })).not.toHaveAttribute(
			'aria-current',
		);
		expect(screen.getByRole('link', { name: 'Documents' })).toHaveAttribute(
			'aria-current',
			'page',
		);
	});

	it('marks Settings current on the settings page', () => {
		renderWorkspace('/collections/col1/settings');

		expect(screen.getByRole('link', { name: 'Settings' })).toHaveAttribute(
			'aria-current',
			'page',
		);
	});
});
