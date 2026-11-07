import { describe, it, expect, afterEach, vi } from 'vitest';
import { screen, waitFor, fireEvent } from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';

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

let cleanupFresh: (() => void) | undefined;

/**
 * The collections store self-initializes on import, so each test re-imports
 * the page with a fresh module graph after installing its IPC mock.
 */
async function renderPage() {
	vi.resetModules();
	listeners.clear();
	const [{ default: Page }, { render, cleanup }] = await Promise.all([
		import('./+page.svelte'),
		import('@testing-library/svelte'),
	]);
	cleanupFresh = cleanup;
	render(Page);
}

afterEach(() => {
	cleanupFresh?.();
	cleanupFresh = undefined;
});

describe('Collections landing page', () => {
	it('shows loading instead of the empty state while collections load', async () => {
		let resolveCollections: (value: unknown[]) => void = () => {};
		mockIPC((cmd) => {
			if (cmd === 'get_collections')
				return new Promise((resolve) => {
					resolveCollections = resolve;
				});
			if (cmd === 'get_pipeline_progress') return [];
		});

		await renderPage();

		expect(
			await screen.findByRole('status', { name: 'Loading collections' }),
		).toBeInTheDocument();
		expect(screen.queryByText('No collections yet')).not.toBeInTheDocument();

		resolveCollections([]);
		await waitFor(() => {
			expect(screen.getByText('No collections yet')).toBeInTheDocument();
		});
	});

	it('shows an error instead of the empty state when loading fails', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_collections') throw new Error('disk unavailable');
			if (cmd === 'get_pipeline_progress') return [];
		});

		await renderPage();

		expect(
			await screen.findByText('Failed to load collections: disk unavailable'),
		).toBeInTheDocument();
		expect(screen.queryByText('No collections yet')).not.toBeInTheDocument();
	});

	it('offers collection creation in the genuine empty state', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_collections') return [];
			if (cmd === 'get_pipeline_progress') return [];
		});

		await renderPage();

		expect(await screen.findByText('No collections yet')).toBeInTheDocument();
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
			if (cmd === 'get_collections') return [collection];
			if (cmd === 'get_pipeline_progress') return [];
			if (cmd === 'share_collection') return 'ticket-123';
		});

		await renderPage();

		expect(
			await screen.findByRole('heading', { name: 'Climate Research' }),
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
		let collectionCalls = 0;
		let resolveRefresh: (value: unknown[]) => void = () => {};
		mockIPC((cmd) => {
			if (cmd === 'get_collections') {
				collectionCalls += 1;
				if (collectionCalls === 1) return [collection];
				return new Promise((resolve) => {
					resolveRefresh = resolve;
				});
			}
			if (cmd === 'get_documents') return [];
			if (cmd === 'get_pipeline_progress') return [];
		});

		await renderPage();

		await screen.findByRole('heading', { name: 'Climate Research' });
		emit('collection-invalidated', { collection_id: 'col1' });
		await waitFor(() => expect(collectionCalls).toBe(2));

		expect(
			screen.getByRole('heading', { name: 'Climate Research' }),
		).toBeInTheDocument();
		expect(screen.queryByRole('status')).not.toBeInTheDocument();

		resolveRefresh([{ ...collection, document_count: 3 }]);
		await waitFor(() => {
			expect(screen.getByText('3 documents')).toBeInTheDocument();
		});
	});

	it('retains loaded cards and shows the error when a background refresh fails', async () => {
		let collectionCalls = 0;
		mockIPC((cmd) => {
			if (cmd === 'get_collections') {
				collectionCalls += 1;
				if (collectionCalls === 1) return [collection];
				throw new Error('sync unavailable');
			}
			if (cmd === 'get_documents') return [];
			if (cmd === 'get_pipeline_progress') return [];
		});

		await renderPage();

		await screen.findByRole('heading', { name: 'Climate Research' });
		emit('collection-invalidated', { collection_id: 'col1' });

		expect(
			await screen.findByText('Failed to load collections: sync unavailable'),
		).toBeInTheDocument();
		expect(
			screen.getByRole('heading', { name: 'Climate Research' }),
		).toBeInTheDocument();
		expect(screen.queryByRole('status')).not.toBeInTheDocument();
	});

	it('states the deletion consequence before confirming', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_collections') return [collection];
			if (cmd === 'get_pipeline_progress') return [];
		});

		await renderPage();

		await screen.findByRole('heading', { name: 'Climate Research' });
		await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));

		const dialog = await screen.findByRole('dialog', {
			name: 'Delete collection',
		});
		expect(dialog).toHaveTextContent(
			'This deletes the collection, its documents, and its local conversations.',
		);
	});
});
