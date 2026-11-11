import { describe, it, expect, vi } from 'vitest';
import {
	render,
	screen,
	fireEvent,
	waitFor,
	within,
} from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
import { goto, invalidate } from '$app/navigation';
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

const providerStatus = { family: null, model: null, error: null };

function renderPage(collections = [collection]) {
	render(Page, {
		props: {
			params: {},
			data: {
				collections,
				provider: { current: null, status: providerStatus },
				providerError: null,
			},
		},
	});
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

describe('Collection import', () => {
	async function openImportDialog(ticket: string) {
		await fireEvent.click(screen.getByRole('button', { name: 'Import' }));
		await fireEvent.input(screen.getByPlaceholderText('Paste ticket here...'), {
			target: { value: ticket },
		});
	}

	function clickConfirmImport() {
		return fireEvent.click(
			within(screen.getByRole('dialog')).getByRole('button', {
				name: 'Import',
			}),
		);
	}

	it('navigates to the imported collection’s documents', async () => {
		mockIPC((cmd) => {
			if (cmd === 'import_collection') return collection;
		});

		renderPage();
		await openImportDialog('ticket-123');
		await clickConfirmImport();

		await waitFor(() =>
			expect(goto).toHaveBeenCalledWith('/collections/col1/documents'),
		);
		expect(invalidate).toHaveBeenCalledWith('app:collections');
	});

	it('does not navigate or clear the reopened modal when a pending import completes after close', async () => {
		let resolveImport: ((value: unknown) => void) | undefined;
		mockIPC((cmd) => {
			if (cmd === 'import_collection')
				return new Promise((resolve) => {
					resolveImport = resolve;
				});
		});

		renderPage();
		await openImportDialog('ticket-123');
		await clickConfirmImport();
		expect(
			screen.getByRole('button', { name: 'Importing...' }),
		).toBeInTheDocument();

		await fireEvent.click(screen.getByRole('button', { name: 'Close dialog' }));
		await fireEvent.click(screen.getByRole('button', { name: 'Import' }));

		resolveImport!(collection);
		await new Promise((resolve) => setTimeout(resolve, 0));

		expect(goto).not.toHaveBeenCalled();
		expect(
			screen.getByRole('dialog', { name: 'Import shared collection' }),
		).toBeInTheDocument();
		expect(screen.getByDisplayValue('ticket-123')).toBeInTheDocument();
	});

	it('shows why a ticket could not be imported', async () => {
		mockIPC((cmd) => {
			if (cmd === 'import_collection')
				throw { code: 'sharing', message: 'bad ticket signature' };
		});

		renderPage();
		await openImportDialog('ticket-123');
		await clickConfirmImport();

		expect(await screen.findByRole('alert')).toHaveTextContent(
			'Failed to import collection: bad ticket signature',
		);
		expect(
			screen.getByRole('dialog', { name: 'Import shared collection' }),
		).toBeInTheDocument();
	});
});

describe('Collections landing page', () => {
	it('links to global settings from the header', () => {
		renderPage();

		expect(screen.getByRole('link', { name: 'Settings' })).toHaveAttribute(
			'href',
			'/settings',
		);
	});

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

	it('opens the research workspace from a card', async () => {
		renderPage();

		expect(
			screen.getByRole('heading', { name: 'Climate Research' }),
		).toBeInTheDocument();
		const workspaceLink = screen.getByRole('link', {
			name: /Climate Research/,
		});
		expect(workspaceLink).toHaveAttribute('href', '/collections/col1');
		expect(screen.getByText('2 documents')).toBeInTheDocument();
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
});

describe('Collection creation', () => {
	it('creates one collection when the form is submitted twice', async () => {
		const created: string[] = [];
		mockIPC((cmd, args) => {
			if (cmd === 'create_collection') {
				created.push((args as { name: string }).name);
				return { ...collection, id: 'new1', document_count: 0 };
			}
		});

		renderPage();
		const field = screen.getByPlaceholderText('New collection...');
		const form = field.closest('form')!;

		await fireEvent.input(field, {
			target: { value: 'Machine Learning' },
		});
		await fireEvent.submit(form);
		await fireEvent.submit(form);

		expect(created).toEqual(['Machine Learning']);
	});
});
