import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import Breadcrumb from './Breadcrumb.svelte';

describe('Breadcrumb', () => {
	it('renders segments', () => {
		render(Breadcrumb, {
			props: {
				segments: [{ label: 'Home', href: '/' }, { label: 'Documents' }],
			},
		});

		expect(screen.getByText('Home')).toBeInTheDocument();
		expect(screen.getByText('Documents')).toBeInTheDocument();
	});

	it('renders links for segments with href', () => {
		render(Breadcrumb, {
			props: {
				segments: [{ label: 'Home', href: '/' }],
			},
		});

		const link = screen.getByRole('link', { name: 'Home' });
		expect(link).toHaveAttribute('href', '/');
	});

	it('renders separators between segments', () => {
		render(Breadcrumb, {
			props: {
				segments: [{ label: 'A' }, { label: 'B' }, { label: 'C' }],
			},
		});

		const separators = screen.getAllByText('/');
		expect(separators).toHaveLength(2);
	});
});
