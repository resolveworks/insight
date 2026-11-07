import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
import { invalidate } from '$app/navigation';
import { page } from '$app/state';
import Page from './+page.svelte';
import AppError from '../+error.svelte';
import { load } from './+layout';
import { startInvalidationListeners } from '#lib/invalidation.js';

type EventListener = (event: { payload: unknown }) => void;
const listeners = new Map<string, EventListener>();

vi.mock('@tauri-apps/api/event', () => ({
	listen: vi.fn(async (name: string, cb: EventListener) => {
		listeners.set(name, cb);
		return () => listeners.delete(name);
	}),
}));

function emit(name: string, payload: unknown) {
	listeners.get(name)?.({ payload });
}

const collection = {
	id: 'col1',
	name: 'Climate Research',
	document_count: 2,
	total_pages: 10,
};

function renderPage(collections = [collection]) {
	render(Page, { props: { params: {}, data: { collections } } });
}

describe('Collections loading', () => {
	it('registers its dependency and returns the collection list', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_collections') return [collection];
		});
		const depends = vi.fn();

		const data = (await load({
			depends,
		} as unknown as Parameters<typeof load>[0])) as { collections: unknown[] };

		expect(depends).toHaveBeenCalledWith('app:collections');
		expect(data.collections).toEqual([collection]);
	});
});

describe('Collections landing page', () => {
	it('offers collection creation in the genuine empty state', () => {
		renderPage([]);

		expect(screen.getByText('No collections yet')).toBeInTheDocument();
		expect(
			screen.getByText('Create one above to start organizing your documents.'),
		).toBeInTheDocument();
		expect(
			screen.getByPlaceholderText('New collection...'),
		).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Create' })).toBeInTheDocument();
	});

	it('opens the research workspace from a card with labeled actions', async () => {
		mockIPC((cmd) => {
			if (cmd === 'share_collection') return 'ticket-123';
		});

		renderPage();

		expect(
			screen.getByRole('heading', { name: 'Climate Research' }),
		).toBeInTheDocument();
		const workspaceLink = screen.getByRole('link', {
			name: /Open research workspace/,
		});
		expect(workspaceLink).toHaveAttribute('href', '/collections/col1');
		expect(screen.getByText('2 documents')).toBeInTheDocument();

		const share = screen.getByRole('button', { name: 'Share' });
		const del = screen.getByRole('button', { name: 'Delete' });
		expect(share.closest('a')).toBeNull();
		expect(del.closest('a')).toBeNull();

		await fireEvent.click(share);
		expect(
			await screen.findByText('Access cannot be revoked once shared.'),
		).toBeInTheDocument();
	});

	it('keeps loaded cards visible while a background refresh is pending', async () => {
		vi.mocked(invalidate).mockImplementationOnce(
			() => new Promise<void>(() => {}),
		);

		renderPage();
		await startInvalidationListeners();

		emit('collection-invalidated', { collection_id: 'col1' });

		expect(invalidate).toHaveBeenCalledWith('app:collections');
		expect(
			screen.getByRole('heading', { name: 'Climate Research' }),
		).toBeInTheDocument();
		expect(screen.getByText('2 documents')).toBeInTheDocument();
		expect(screen.queryByRole('status')).not.toBeInTheDocument();
	});

	it('shows a collection list load failure and retries it', async () => {
		page.status = 500;
		page.error = { message: 'disk unavailable', status: 500 };

		render(AppError);

		expect(
			screen.getByText('Failed to load collections: disk unavailable'),
		).toBeInTheDocument();

		await fireEvent.click(screen.getByRole('button', { name: 'Retry' }));

		expect(invalidate).toHaveBeenLastCalledWith('app:collections');
	});

	it('states the deletion consequence before confirming', async () => {
		renderPage();

		await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));

		const dialog = await screen.findByRole('dialog', {
			name: 'Delete collection',
		});
		expect(dialog).toHaveTextContent(
			'This deletes the collection, its documents, and its local conversations.',
		);
	});
});
