import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
	render,
	screen,
	fireEvent,
	waitFor,
	within,
} from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
import { open } from '@tauri-apps/plugin-dialog';
import { invalidate } from '$app/navigation';
import { dataKeys } from '#lib/invalidation.js';
import Page from './+page.svelte';
import { load } from './+page';

type EventListener = (event: { payload: unknown }) => void;
const listeners = new Map<string, EventListener>();

vi.mock('@tauri-apps/api/event', () => ({
	listen: vi.fn(async (name: string, cb: EventListener) => {
		listeners.set(name, cb);
		return () => listeners.delete(name);
	}),
}));

vi.mock('@tauri-apps/plugin-dialog', () => ({
	open: vi.fn(),
}));

function emit(name: string, payload: unknown) {
	listeners.get(name)?.({ payload });
}

const collection = {
	id: 'col1',
	name: 'Climate Research',
	document_count: 1,
	total_pages: 12,
};

const providerStatus = { family: null, model: null, error: null };

const document = {
	id: 'doc1',
	name: 'report.pdf',
	file_type: 'application/pdf',
	page_count: 12,
	tags: [],
	created_at: '2024-01-15T10:30:00Z',
};

const deletedDocument = {
	id: 'doc2',
	name: 'archived.pdf',
	deleted_by: 'a1b2c3d4e5f6a7b8c9d0',
	deleted_at: '2024-02-20T14:45:00Z',
};

const activeProgress = {
	collection_id: 'col1',
	store: { pending: 0, active: 0, completed: 1, failed: 0 },
	extract: { pending: 1, active: 1, completed: 0, failed: 0 },
	index: { pending: 0, active: 0, completed: 0, failed: 0 },
};

const idleProgress = {
	collection_id: 'col1',
	store: { pending: 0, active: 0, completed: 1, failed: 0 },
	extract: { pending: 0, active: 0, completed: 2, failed: 0 },
	index: { pending: 0, active: 0, completed: 2, failed: 0 },
};

beforeEach(() => {
	listeners.clear();
	vi.mocked(open).mockReset();
	mockIPC((cmd) => {
		if (cmd === 'get_pipeline_progress') return [];
	});
});

function renderPage(
	documents = [document],
	deletedDocuments: (typeof deletedDocument)[] = [],
) {
	render(Page, {
		props: {
			params: { collectionId: 'col1' },
			data: {
				collections: [collection],
				collection,
				documents,
				deletedDocuments,
				provider: { current: null, status: providerStatus },
				providerError: null,
			},
		},
	});
}

function deletedItemsSection(): HTMLElement {
	return screen
		.getByRole('heading', { name: 'Deleted items' })
		.closest('section') as HTMLElement;
}

describe('Collection documents load', () => {
	it('registers its dependency and returns the active and deleted documents', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_documents') return [document];
			if (cmd === 'get_deleted_documents') return [deletedDocument];
		});
		const depends = vi.fn();

		const data = (await load({
			params: { collectionId: 'col1' },
			depends,
		} as unknown as Parameters<typeof load>[0])) as {
			documents: unknown[];
			deletedDocuments: unknown[];
		};

		expect(depends).toHaveBeenCalledWith(dataKeys.collection('col1'));
		expect(data.documents).toEqual([document]);
		expect(data.deletedDocuments).toEqual([deletedDocument]);
	});
});

describe('Collection documents page', () => {
	it('explains the empty collection and offers importing a PDF', () => {
		renderPage([]);

		expect(screen.getByText('No documents yet')).toBeInTheDocument();
		expect(
			screen.getByText(
				'Documents you import or that arrive from colleagues syncing this collection will appear here as they become available.',
			),
		).toBeInTheDocument();
		expect(
			screen.getByRole('button', { name: 'Import PDF' }),
		).toBeInTheDocument();
	});

	it('shows each document as a link with a labeled delete action', async () => {
		renderPage();

		const link = screen.getByRole('link', { name: /report\.pdf/ });
		expect(link).toHaveAttribute('href', '/collections/col1/documents/doc1');
		expect(screen.getByText('12 pages')).toBeInTheDocument();

		const del = screen.getByRole('button', { name: 'Delete document' });
		expect(del.closest('a')).toBeNull();
		expect(screen.getAllByRole('button', { name: 'Import PDF' })).toHaveLength(
			1,
		);

		await fireEvent.click(del);
		expect(
			screen.getByRole('dialog', { name: 'Delete document' }),
		).toHaveTextContent('You can restore it from Deleted items.');
	});

	it('renders active documents separately from plain-text deleted items', () => {
		renderPage([document], [deletedDocument]);

		expect(
			screen.getByRole('link', { name: /report\.pdf/ }),
		).toBeInTheDocument();

		const section = deletedItemsSection();
		const deletedName = within(section).getByText('archived.pdf');
		expect(deletedName.closest('a')).toBeNull();
		expect(within(section).getByText(/Deleted by a1b2c3d4…/)).toHaveTextContent(
			new Intl.DateTimeFormat(undefined, {
				dateStyle: 'medium',
				timeStyle: 'short',
			}).format(new Date(deletedDocument.deleted_at)),
		);
		expect(
			within(section).getByRole('button', { name: 'Restore' }),
		).toBeInTheDocument();
	});

	it('shows the deleted items state when every document is deleted', () => {
		renderPage([], [deletedDocument]);

		expect(screen.getByText('No active documents')).toBeInTheDocument();
		expect(
			screen.getByText(
				'Every document in this collection is in Deleted items.',
			),
		).toBeInTheDocument();
		expect(screen.queryByText('No documents yet')).not.toBeInTheDocument();
		expect(
			within(deletedItemsSection()).getByText('archived.pdf'),
		).toBeInTheDocument();
	});

	it('restores a deleted document by id and re-reads the collection', async () => {
		const calls: unknown[] = [];
		mockIPC((cmd, args) => {
			if (cmd === 'get_pipeline_progress') return [];
			if (cmd === 'undelete_document') {
				calls.push(args);
				return null;
			}
		});

		renderPage([], [deletedDocument]);
		await fireEvent.click(screen.getByRole('button', { name: 'Restore' }));

		await waitFor(() =>
			expect(invalidate).toHaveBeenCalledWith('app:collection:col1'),
		);
		expect(invalidate).toHaveBeenCalledWith('app:collections');
		expect(calls).toEqual([{ collectionId: 'col1', documentId: 'doc2' }]);
	});

	it('explains that deletion is reversible and visible to peers', async () => {
		renderPage();
		await fireEvent.click(
			screen.getByRole('button', { name: 'Delete document' }),
		);

		const dialog = screen.getByRole('dialog', { name: 'Delete document' });
		expect(dialog).toHaveTextContent(
			'Everyone syncing this collection will stop seeing it in the active list and search.',
		);
		expect(dialog).toHaveTextContent('You can restore it from Deleted items.');
		expect(dialog).not.toHaveTextContent('extracted text');
	});

	it('lists the files an import could not store', async () => {
		vi.mocked(open).mockResolvedValue(['/tmp/report.pdf']);
		mockIPC((cmd) => {
			if (cmd === 'get_pipeline_progress') return [];
			if (cmd === 'start_import')
				return {
					documents: [],
					failures: [{ path: '/tmp/report.pdf', message: 'Not a valid PDF' }],
				};
		});

		renderPage();
		await fireEvent.click(screen.getByRole('button', { name: 'Import PDF' }));

		expect(
			await screen.findByText('Some files could not be imported.'),
		).toBeInTheDocument();
		expect(
			screen.getByText('/tmp/report.pdf: Not a valid PDF'),
		).toBeInTheDocument();
	});

	it('shows why an import could not be started', async () => {
		vi.mocked(open).mockResolvedValue(['/tmp/report.pdf']);
		mockIPC((cmd) => {
			if (cmd === 'get_pipeline_progress') return [];
			if (cmd === 'start_import') throw new Error('disk full');
		});

		renderPage();
		await fireEvent.click(screen.getByRole('button', { name: 'Import PDF' }));

		expect(await screen.findByText('disk full')).toBeInTheDocument();
	});

	it('shows pipeline progress and returns to the empty state when it goes idle', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_pipeline_progress') return [activeProgress];
		});

		renderPage([]);

		expect(
			await screen.findByText('Processing documents…'),
		).toBeInTheDocument();
		expect(screen.getByText('Extract')).toBeInTheDocument();
		expect(screen.getByText('0/2')).toBeInTheDocument();

		emit('pipeline-progress', idleProgress);

		expect(await screen.findByText('No documents yet')).toBeInTheDocument();
		expect(screen.queryByText('Extract')).not.toBeInTheDocument();
	});
});
