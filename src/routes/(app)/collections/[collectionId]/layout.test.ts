import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import { page } from '$app/state';
import Layout from './+layout.svelte';
import { load } from './+layout';

const collection = {
	id: 'col1',
	name: 'Climate Research',
	document_count: 2,
	total_pages: 10,
};

describe('Collection workspace shell', () => {
	it.each([
		['/(app)/collections/[collectionId]/research', 'Research'],
		['/(app)/collections/[collectionId]/documents', 'Documents'],
		['/(app)/collections/[collectionId]/documents/[documentId]', 'Documents'],
		['/(app)/collections/[collectionId]/settings', 'Settings'],
	] as const)('renders one persistent shell for %s', (routeId, activeLabel) => {
		page.route.id = routeId;
		const children = createRawSnippet(() => ({
			render: () => '<p>Route content</p>',
		}));

		render(Layout, {
			props: {
				data: {
					provider: null,
					providerError: null,
					collections: [collection],
					collection,
				},
				children,
				params: { collectionId: collection.id },
			},
		});

		expect(screen.getAllByRole('banner')).toHaveLength(1);
		expect(
			screen.getByRole('heading', { level: 1, name: 'Climate Research' }),
		).toBeInTheDocument();
		expect(screen.getByText('Route content')).toBeInTheDocument();
		expect(screen.getByRole('link', { name: activeLabel })).toHaveAttribute(
			'aria-current',
			'page',
		);
	});
});

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
