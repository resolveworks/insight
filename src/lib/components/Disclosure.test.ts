import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import { createRawSnippet, tick } from 'svelte';
import Disclosure from './Disclosure.svelte';

const summary = () =>
	createRawSnippet(() => ({ render: () => `<span>Summary</span>` }));
const body = () =>
	createRawSnippet(() => ({ render: () => `<p>Hidden content</p>` }));

describe('Disclosure', () => {
	it('keeps its content collapsed until the summary is clicked', async () => {
		const { container } = render(Disclosure, {
			props: { summary: summary(), children: body() },
		});

		const details = container.querySelector('details')!;
		expect(details.open).toBe(false);
		expect(screen.getByText('Hidden content')).not.toBeVisible();

		screen.getByText('Summary').click();
		await tick();

		expect(details.open).toBe(true);
		expect(screen.getByText('Hidden content')).toBeVisible();
	});

	it('reflects the toggled state in a bound open prop', async () => {
		let open = false;
		render(Disclosure, {
			props: {
				get open() {
					return open;
				},
				set open(value: boolean) {
					open = value;
				},
				summary: summary(),
				children: body(),
			},
		});

		expect(open).toBe(false);

		screen.getByText('Summary').click();
		await new Promise((resolve) => setTimeout(resolve, 0));

		expect(open).toBe(true);
	});
});
