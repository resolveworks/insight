import { describe, it, expect, afterEach } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import { page } from '$app/state';
import Layout from './+layout.svelte';

describe('App layout navigation', () => {
	afterEach(() => {
		page.url = new URL('http://localhost/collections');
		page.route.id = '/(app)/collections';
	});

	it('links to every section', () => {
		page.url = new URL('http://localhost/collections/abc/research');
		page.route.id = '/(app)/collections/[collectionId]/research';

		render(Layout, {
			props: {
				children: createRawSnippet(() => ({ render: () => '<span></span>' })),
			},
		});

		const links = screen.getAllByRole('link');
		expect(links.map((link) => link.textContent?.trim())).toEqual([
			'Collections',
			'Settings',
		]);
		expect(links.map((link) => link.getAttribute('href'))).toEqual([
			'/collections',
			'/settings',
		]);
		expect(links[0]).toHaveAttribute('aria-current', 'page');
		expect(links[1]).not.toHaveAttribute('aria-current');
	});

	it('marks Settings current outside the collections section', () => {
		page.url = new URL('http://localhost/settings');
		page.route.id = '/(app)/settings';

		render(Layout, {
			props: {
				children: createRawSnippet(() => ({ render: () => '<span></span>' })),
			},
		});

		const links = screen.getAllByRole('link');
		expect(links[0]).not.toHaveAttribute('aria-current');
		expect(links[1]).toHaveAttribute('aria-current', 'page');
	});
});
