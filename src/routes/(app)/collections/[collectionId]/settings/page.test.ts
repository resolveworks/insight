import { describe, it, expect, vi } from 'vitest';
import {
	render,
	screen,
	fireEvent,
	waitFor,
	within,
} from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
import { goto } from '$app/navigation';
import Page from './+page.svelte';

const collection = {
	id: 'col1',
	name: 'Climate Research',
	document_count: 2,
	total_pages: 10,
};

const providerStatus = { family: null, model: null, error: null };

function renderPage() {
	render(Page, {
		props: {
			params: { collectionId: 'col1' },
			data: {
				collections: [collection],
				collection,
				documents: [],
				provider: { current: null, status: providerStatus },
				providerError: null,
			},
		},
	});
}

describe('Collection settings', () => {
	it('shares the collection with a ticket that cannot be revoked', async () => {
		mockIPC((cmd) => {
			if (cmd === 'share_collection') return 'ticket-123';
		});

		renderPage();
		await fireEvent.click(screen.getByRole('button', { name: 'Share' }));

		expect(await screen.findByText('ticket-123')).toBeInTheDocument();
		expect(
			screen.getByText('Access cannot be revoked once shared.'),
		).toBeInTheDocument();
	});

	it('copies the share ticket to the clipboard', async () => {
		const writeText = vi.fn(async () => {});
		Object.defineProperty(navigator, 'clipboard', {
			value: { writeText },
			configurable: true,
		});
		mockIPC((cmd) => {
			if (cmd === 'share_collection') return 'ticket-123';
		});

		renderPage();
		await fireEvent.click(screen.getByRole('button', { name: 'Share' }));
		await fireEvent.click(await screen.findByRole('button', { name: 'Copy' }));

		expect(writeText).toHaveBeenCalledWith('ticket-123');
		expect(
			await screen.findByRole('button', { name: 'Copied!' }),
		).toBeInTheDocument();
	});

	it('states the deletion consequence and returns to the collection list', async () => {
		const deleted: unknown[] = [];
		mockIPC((cmd, payload) => {
			if (cmd === 'delete_collection') {
				deleted.push(payload);
				return undefined;
			}
		});

		renderPage();
		await fireEvent.click(
			screen.getByRole('button', { name: 'Delete collection' }),
		);

		const dialog = await screen.findByRole('dialog', {
			name: 'Delete collection',
		});
		expect(dialog).toHaveTextContent(
			'This deletes the collection, its documents, and its local conversations.',
		);

		await fireEvent.click(
			within(dialog).getByRole('button', { name: 'Delete' }),
		);

		await waitFor(() => expect(goto).toHaveBeenCalledWith('/collections'));
		expect(deleted).toEqual([{ collectionId: 'col1' }]);
	});
});
