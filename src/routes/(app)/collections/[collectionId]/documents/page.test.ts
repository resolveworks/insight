import { describe, it, expect, afterEach, vi } from 'vitest';
import { screen, waitFor, fireEvent } from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';

vi.mock('@tauri-apps/api/event', () => ({
	listen: vi.fn(async () => () => {}),
}));

const collection = {
	id: 'col1',
	name: 'Climate Research',
	document_count: 1,
	total_pages: 12,
};

const document = {
	id: 'doc1',
	name: 'report.pdf',
	file_type: 'application/pdf',
	page_count: 12,
	tags: [],
	created_at: '2024-01-15T10:30:00Z',
};

let cleanupFresh: (() => void) | undefined;

/**
 * The collections store self-initializes on import, so each test re-imports
 * the page with a fresh module graph after installing its IPC mock.
 */
async function renderPage() {
	vi.resetModules();
	const [{ default: Page }, { render, cleanup }, { page }] = await Promise.all([
		import('./+page.svelte'),
		import('@testing-library/svelte'),
		import('$app/state'),
	]);
	page.url = new URL('http://localhost/collections/col1/documents');
	page.params = { collectionId: 'col1' };
	cleanupFresh = cleanup;
	render(Page);
}

afterEach(() => {
	cleanupFresh?.();
	cleanupFresh = undefined;
});

describe('Collection documents page', () => {
	it('shows loading instead of the empty state until documents arrive', async () => {
		let resolveDocuments: (value: unknown[]) => void = () => {};
		mockIPC((cmd) => {
			if (cmd === 'get_collections') return [collection];
			if (cmd === 'get_documents')
				return new Promise((resolve) => {
					resolveDocuments = resolve;
				});
			if (cmd === 'get_pipeline_progress') return [];
		});

		await renderPage();

		expect(await screen.findByRole('status')).toHaveTextContent(
			'Loading documents',
		);
		expect(screen.queryByText('No documents yet')).not.toBeInTheDocument();

		resolveDocuments([]);
		await waitFor(() => {
			expect(screen.getByText('No documents yet')).toBeInTheDocument();
		});
	});

	it('replaces the spinner with the load error and retries into the empty state', async () => {
		let attempts = 0;
		mockIPC((cmd) => {
			if (cmd === 'get_collections') return [collection];
			if (cmd === 'get_documents') {
				attempts += 1;
				if (attempts === 1) throw new Error('storage unavailable');
				return [];
			}
			if (cmd === 'get_pipeline_progress') return [];
		});

		await renderPage();

		expect(
			await screen.findByText('Failed to load documents: storage unavailable'),
		).toBeInTheDocument();
		expect(screen.queryByRole('status')).not.toBeInTheDocument();

		await fireEvent.click(screen.getByRole('button', { name: 'Retry' }));

		expect(await screen.findByText('No documents yet')).toBeInTheDocument();
		expect(
			screen.queryByText(/Failed to load documents/),
		).not.toBeInTheDocument();
		expect(attempts).toBe(2);
	});

	it('retries a failed load into the document list', async () => {
		let attempts = 0;
		mockIPC((cmd) => {
			if (cmd === 'get_collections') return [collection];
			if (cmd === 'get_documents') {
				attempts += 1;
				if (attempts === 1) throw new Error('storage unavailable');
				return [document];
			}
			if (cmd === 'get_pipeline_progress') return [];
		});

		await renderPage();

		await screen.findByText('Failed to load documents: storage unavailable');
		await fireEvent.click(screen.getByRole('button', { name: 'Retry' }));

		expect(
			await screen.findByRole('link', { name: /report\.pdf/ }),
		).toHaveAttribute('href', '/collections/col1/documents/doc1');
		expect(
			screen.queryByText(/Failed to load documents/),
		).not.toBeInTheDocument();
	});

	it('explains the empty collection and offers importing a PDF', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_collections') return [collection];
			if (cmd === 'get_documents') return [];
			if (cmd === 'get_pipeline_progress') return [];
		});

		await renderPage();

		expect(await screen.findByText('No documents yet')).toBeInTheDocument();
		expect(
			screen.getByText(
				'The research agent answers from documents in this collection. Import a PDF to give it sources.',
			),
		).toBeInTheDocument();
		expect(
			screen.getByRole('button', { name: 'Import PDF' }),
		).toBeInTheDocument();
	});

	it('shows each document as a link with a labeled delete action', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_collections') return [collection];
			if (cmd === 'get_documents') return [document];
			if (cmd === 'get_pipeline_progress') return [];
		});

		await renderPage();

		const link = await screen.findByRole('link', { name: /report\.pdf/ });
		expect(link).toHaveAttribute('href', '/collections/col1/documents/doc1');
		expect(screen.getByText('12 pages')).toBeInTheDocument();

		const del = screen.getByRole('button', { name: 'Delete' });
		expect(del.closest('a')).toBeNull();
		expect(screen.getAllByRole('button', { name: 'Import PDF' })).toHaveLength(
			1,
		);

		await fireEvent.click(del);
		expect(
			await screen.findByRole('dialog', { name: 'Delete document' }),
		).toHaveTextContent('from this collection');
	});
});
