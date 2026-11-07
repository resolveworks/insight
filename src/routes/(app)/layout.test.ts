import { describe, it, expect, afterEach } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import { page } from '$app/state';
import Layout from './+layout.svelte';

describe('App layout navigation', () => {
	afterEach(() => {
		page.url = new URL('http://localhost/files');
	});

	it('links to every section', () => {
		page.url = new URL('http://localhost/files/abc');

		render(Layout, {
			props: {
				children: createRawSnippet(() => ({ render: () => '<span></span>' })),
			},
		});

		const links = screen.getAllByRole('link');
		expect(links.map((link) => link.textContent?.trim())).toEqual([
			'Files',
			'Settings',
		]);
		expect(links.map((link) => link.getAttribute('href'))).toEqual([
			'/files',
			'/settings',
		]);
	});
});
