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
import { load } from './+page';
import type { CollectionSharingStatus } from '#lib/types/sharing.js';

const collection = {
	id: 'col1',
	name: 'Climate Research',
	document_count: 2,
	total_pages: 10,
};

const providerStatus = { family: null, model: null, error: null };

const sharingStatus: CollectionSharingStatus = {
	sync_running: true,
	network: 'online',
	connected_peers: 1,
	known_peers: 3,
	last_sync: {
		peer: 'a1b2c3d4',
		finished_at: '2024-01-15T10:30:00Z',
		entries_received: 4,
		entries_sent: 2,
		error: null,
	},
};

function renderPage(sharing = sharingStatus) {
	render(Page, {
		props: {
			params: { collectionId: 'col1' },
			data: {
				collections: [collection],
				collection,
				documents: [],
				provider: { current: null, status: providerStatus },
				providerError: null,
				sharing,
			},
		},
	});
}

describe('Collection settings sharing load', () => {
	it('registers its dependency and returns the sharing status', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_collection_sharing_status') return sharingStatus;
		});
		const depends = vi.fn();

		const data = (await load({
			params: { collectionId: 'col1' },
			depends,
		} as unknown as Parameters<typeof load>[0])) as {
			sharing: CollectionSharingStatus;
		};

		expect(depends).toHaveBeenCalledWith('app:sharing');
		expect(data.sharing).toEqual(sharingStatus);
	});
});

describe('Collection settings sharing status', () => {
	it('shows the current sharing status and last successful sync', () => {
		renderPage();

		expect(screen.queryByText('Access')).not.toBeInTheDocument();
		expect(screen.getByText('Sync')).toBeInTheDocument();
		expect(screen.getByText('Running')).toBeInTheDocument();
		expect(screen.getByText('Relays')).toBeInTheDocument();
		expect(screen.getByText('Connected')).toBeInTheDocument();
		expect(
			screen.getByText('1 connected · 3 previously synced'),
		).toBeInTheDocument();

		const finishedAt = new Intl.DateTimeFormat(undefined, {
			dateStyle: 'medium',
			timeStyle: 'short',
		}).format(new Date('2024-01-15T10:30:00Z'));
		expect(screen.getByText(finishedAt, { exact: false })).toBeInTheDocument();
		expect(screen.getByText('a1b2c3d4')).toBeInTheDocument();
		expect(screen.getByText('4 received · 2 sent')).toBeInTheDocument();
	});

	it('shows a not-running, offline, never-synced state', () => {
		renderPage({
			...sharingStatus,
			sync_running: false,
			network: 'offline',
			connected_peers: 0,
			known_peers: 2,
			last_sync: null,
		});

		expect(screen.getByText('Not running')).toBeInTheDocument();
		expect(screen.getByText('Disconnected')).toBeInTheDocument();
		expect(
			screen.getByText('0 connected · 2 previously synced'),
		).toBeInTheDocument();
		expect(
			screen.getByText('No sync completed since startup'),
		).toBeInTheDocument();
	});

	it('shows that the relay connection is being established', () => {
		renderPage({
			...sharingStatus,
			network: 'connecting',
		});

		expect(screen.getByText('Relays')).toBeInTheDocument();
		expect(screen.getByText('Connecting')).toBeInTheDocument();
	});

	it('shows the error when the last sync failed', () => {
		renderPage({
			...sharingStatus,
			last_sync: {
				...sharingStatus.last_sync!,
				error: 'connection reset',
			},
		});

		expect(
			screen.getByText('Sync failed: connection reset'),
		).toBeInTheDocument();
		expect(screen.queryByText('4 received · 2 sent')).not.toBeInTheDocument();
	});
});

describe('Collection settings actions', () => {
	it('creates a collaboration ticket that cannot be revoked', async () => {
		mockIPC((cmd) => {
			if (cmd === 'share_collection') return 'ticket-123';
		});

		renderPage();
		await fireEvent.click(
			screen.getByRole('button', { name: 'Create collaboration ticket' }),
		);

		expect(await screen.findByText('ticket-123')).toBeInTheDocument();
		expect(
			screen.getByText(/Access cannot be revoked once shared/),
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
		await fireEvent.click(
			screen.getByRole('button', { name: 'Create collaboration ticket' }),
		);
		await fireEvent.click(await screen.findByRole('button', { name: 'Copy' }));

		expect(writeText).toHaveBeenCalledWith('ticket-123');
		expect(
			await screen.findByRole('button', { name: 'Copied!' }),
		).toBeInTheDocument();
	});

	it('reports why a share ticket could not be created', async () => {
		mockIPC((cmd) => {
			if (cmd === 'share_collection')
				throw { code: 'sharing', message: 'relay unavailable' };
		});

		renderPage();
		await fireEvent.click(
			screen.getByRole('button', { name: 'Create collaboration ticket' }),
		);

		expect(await screen.findByRole('alert')).toHaveTextContent(
			'Failed to generate share ticket: relay unavailable',
		);
	});

	it('tells the user to copy manually when the clipboard rejects', async () => {
		Object.defineProperty(navigator, 'clipboard', {
			value: {
				writeText: vi.fn(async () => {
					throw new Error('not permitted');
				}),
			},
			configurable: true,
		});
		mockIPC((cmd) => {
			if (cmd === 'share_collection') return 'ticket-123';
		});

		renderPage();
		await fireEvent.click(
			screen.getByRole('button', { name: 'Create collaboration ticket' }),
		);
		await fireEvent.click(await screen.findByRole('button', { name: 'Copy' }));

		expect(await screen.findByRole('alert')).toHaveTextContent(
			"Couldn't copy the ticket. Select it and copy it manually.",
		);
		expect(screen.getByRole('button', { name: 'Copy' })).toBeInTheDocument();
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
