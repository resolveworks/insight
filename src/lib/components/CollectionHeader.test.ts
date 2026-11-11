import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import CollectionHeader from './CollectionHeader.svelte';

const collection = { id: 'col1', name: 'Climate Research' };

const sectionLinks = () => screen.getAllByRole('link').slice(1);

describe('CollectionHeader', () => {
	it('links back to the collections list', () => {
		render(CollectionHeader, { props: { collection, active: 'research' } });

		expect(
			screen.getByRole('link', { name: 'Back to collections' }),
		).toHaveAttribute('href', '/collections');
		expect(
			screen.getByRole('heading', { name: 'Climate Research' }),
		).toBeInTheDocument();
	});

	it('links to every section of the collection', () => {
		render(CollectionHeader, { props: { collection, active: 'research' } });

		const links = sectionLinks();
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
	});

	it('renders compact collection context without a collection heading', () => {
		render(CollectionHeader, {
			props: { collection, active: 'documents', compact: true },
		});

		expect(
			screen.getByRole('link', { name: 'Back to collections' }),
		).toHaveAttribute('href', '/collections');
		expect(screen.getByText('Climate Research')).toBeVisible();
		expect(
			screen.queryByRole('heading', { name: 'Climate Research' }),
		).not.toBeInTheDocument();

		const links = sectionLinks();
		expect(links.map((link) => link.textContent?.trim())).toEqual([
			'Research',
			'Documents',
			'Settings',
		]);
		expect(screen.getByRole('link', { name: 'Documents' })).toHaveAttribute(
			'aria-current',
			'page',
		);
	});

	it.each([
		['research', 'Research'],
		['documents', 'Documents'],
		['settings', 'Settings'],
	] as const)(
		'marks only %s current when it is the active section',
		(active, activeLabel) => {
			render(CollectionHeader, { props: { collection, active } });

			for (const label of ['Research', 'Documents', 'Settings']) {
				const link = screen.getByRole('link', { name: label });
				if (label === activeLabel) {
					expect(link).toHaveAttribute('aria-current', 'page');
				} else {
					expect(link).not.toHaveAttribute('aria-current');
				}
			}
		},
	);
});
