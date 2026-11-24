import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import {
	render,
	screen,
	cleanup,
	fireEvent,
	waitFor,
} from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { mockIPC } from '@tauri-apps/api/mocks';
import { page } from '$app/state';
import { replaceState } from '$app/navigation';
import { load as documentLoad } from './+page';
import DocumentPage from './+page.svelte';
import type { PageProps } from './$types';
import { formatTimestamp } from '#lib/format.js';
import type { Document } from '#lib/types/documents.js';

vi.mock('@tauri-apps/api/event', () => ({
	listen: vi.fn(async () => () => {}),
}));

const collection = {
	id: 'col1',
	name: 'Climate Research',
	document_count: 1,
	total_pages: 12,
};

const documentMeta: Document = {
	id: 'doc1',
	name: 'report.pdf',
	file_type: 'application/pdf',
	page_count: 12,
	partially_indexed: null,
	tags: [],
	created_at: '2024-01-15T10:30:00Z',
};

function renderPage(overrides: Partial<typeof documentMeta> = {}) {
	const pageDocument = { ...documentMeta, ...overrides };
	render(DocumentPage, {
		props: {
			data: {
				collections: [collection],
				collection,
				documents: [pageDocument],
				document: pageDocument,
				provider: null,
				providerError: null,
			} as PageProps['data'],
			params: { collectionId: 'col1', documentId: 'doc1' },
		},
	});
}

beforeEach(() => {
	page.url = new URL('http://localhost/collections/col1/documents/doc1');
});

afterEach(() => {
	cleanup();
	vi.useRealTimers();
	vi.unstubAllGlobals();
	vi.restoreAllMocks();
});

describe('Document detail load', () => {
	it('registers its dependency and returns the document', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document') return documentMeta;
		});
		const depends = vi.fn();

		const data = (await documentLoad({
			params: { collectionId: 'col1', documentId: 'doc1' },
			depends,
		} as unknown as Parameters<typeof documentLoad>[0])) as {
			document: typeof documentMeta;
		};

		expect(depends).toHaveBeenCalledWith('app:collection:col1');
		expect(data.document).toEqual(documentMeta);
	});
});

describe('Document detail page', () => {
	it('names the document with a second-level content heading', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document_text') return 'Extracted text';
		});

		renderPage();

		expect(
			await screen.findByRole('heading', { level: 2, name: 'report.pdf' }),
		).toBeInTheDocument();
		expect(screen.queryByRole('heading', { level: 1 })).not.toBeInTheDocument();
	});

	it('links back to the documents list above the title', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document_text') return 'Extracted text';
		});

		renderPage();

		const back = await screen.findByRole('link', { name: 'Back to documents' });
		expect(back).toHaveAttribute('href', '/collections/col1/documents');
		expect(
			back.compareDocumentPosition(screen.getByRole('heading', { level: 2 })),
		).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
	});

	it('summarises the document under its title', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document_text') return 'Extracted text';
		});

		renderPage({ page_count: 1, tags: ['budget', 'eu'] });

		expect(await screen.findByText('1 page')).toBeVisible();
		expect(
			screen.getByText(`Added ${formatTimestamp(documentMeta.created_at)}`),
		).toBeVisible();
		expect(screen.getByText('budget')).toBeVisible();
	});

	it('renders the extracted document text', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document_text') return 'Extracted text';
		});

		renderPage();

		expect(await screen.findByText('Extracted text')).toBeVisible();
	});

	it('warns that search covers only part of a partially indexed document', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document_text') return 'Extracted text';
		});

		renderPage({ partially_indexed: true });

		expect(await screen.findByText('Partially searchable')).toBeInTheDocument();
		expect(
			screen.getByText(
				'Text near the end of this document is not included in search because it exceeds the indexing limit. The full document is still available.',
			),
		).toBeVisible();
	});

	it('shows no partial-index warning when coverage is full', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document_text') return 'Extracted text';
		});

		renderPage({ partially_indexed: false });

		expect(await screen.findByText('Extracted text')).toBeVisible();
		expect(screen.queryByText('Partially searchable')).not.toBeInTheDocument();
	});

	it('shows no partial-index warning while coverage is unknown', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document_text') return 'Extracted text';
		});

		renderPage({ partially_indexed: null });

		expect(await screen.findByText('Extracted text')).toBeVisible();
		expect(screen.queryByText('Partially searchable')).not.toBeInTheDocument();
	});
});

describe('Document detail open original', () => {
	it('opens the original and disables the button while the request is pending', async () => {
		let resolveOpen!: () => void;
		const opened: unknown[] = [];
		mockIPC((cmd, payload) => {
			if (cmd === 'get_document_text') return 'Extracted text';
			if (cmd === 'open_document_original') {
				opened.push(payload);
				return new Promise<void>((resolve) => {
					resolveOpen = resolve;
				});
			}
		});

		renderPage();

		const open = await screen.findByRole('button', { name: 'Open original' });
		expect(open).toBeEnabled();

		await userEvent.click(open);

		await waitFor(() => expect(open).toBeDisabled());
		expect(opened).toEqual([{ collectionId: 'col1', documentId: 'doc1' }]);

		resolveOpen();
		await waitFor(() => expect(open).toBeEnabled());
	});

	it('shows a clear error when opening the original fails', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document_text') return 'Extracted text';
			if (cmd === 'open_document_original') {
				throw { code: 'not_found', message: 'Original file is missing' };
			}
		});

		renderPage();

		await userEvent.click(
			await screen.findByRole('button', { name: 'Open original' }),
		);

		expect(await screen.findByRole('alert')).toHaveTextContent(
			'Failed to open original: Original file is missing',
		);
	});

	it('stays available when extraction is unavailable', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document_text') {
				throw { code: 'not_extracted', message: 'Extraction is running' };
			}
		});

		renderPage({ partially_indexed: null });

		expect(
			await screen.findByRole('button', { name: 'Open original' }),
		).toBeEnabled();
	});
});

describe('Document detail search', () => {
	const matches = [
		{ content: 'needle', offset: 0, next_offset: 6, source_range: null },
		{ content: 'needle', offset: 13, next_offset: 19, source_range: null },
		{ content: 'needle', offset: 25, next_offset: 31, source_range: null },
	];
	const results = { partially_indexed: false, matches };
	const text = 'needle alpha needle beta needle';

	function mockSearch(handler: (payload: unknown) => unknown = () => results) {
		const searches: unknown[] = [];
		mockIPC((cmd, payload) => {
			if (cmd === 'get_document_text') return text;
			if (cmd === 'search_document') {
				searches.push(payload);
				return handler(payload);
			}
		});
		return searches;
	}

	it('runs a deep-linked search immediately and highlights its hit in the text', async () => {
		page.url = new URL(
			'http://localhost/collections/col1/documents/doc1?q=needle&hit=2',
		);
		const searches = mockSearch();

		renderPage({ partially_indexed: false });

		expect(await screen.findByText('2 of 3')).toBeVisible();
		expect(searches).toEqual([
			{ collectionId: 'col1', documentId: 'doc1', query: 'needle' },
		]);
		const marks = document.querySelectorAll('mark');
		expect(marks).toHaveLength(3);
		expect(marks[1]).toHaveAttribute('data-active', 'true');
	});

	it('searches after manual typing settles and resets the URL hit', async () => {
		mockSearch();
		renderPage({ partially_indexed: false });
		const input = await screen.findByRole('textbox', {
			name: 'Search document',
		});

		vi.useFakeTimers();
		await fireEvent.input(input, { target: { value: 'needle' } });

		expect(page.url.searchParams.get('q')).toBe('needle');
		expect(page.url.searchParams.has('hit')).toBe(false);
		expect(replaceState).toHaveBeenCalled();

		await vi.advanceTimersByTimeAsync(200);

		expect(await screen.findByText('1 of 3')).toBeVisible();
		expect(
			screen.getByRole('textbox', { name: 'Search document' }),
		).toHaveValue('needle');
		vi.useRealTimers();
	});

	it('clears the query without searching and disables navigation', async () => {
		page.url = new URL(
			'http://localhost/collections/col1/documents/doc1?q=needle&hit=2',
		);
		const searches = mockSearch();
		renderPage({ partially_indexed: false });
		expect(await screen.findByText('2 of 3')).toBeVisible();

		await userEvent.click(screen.getByRole('button', { name: 'Clear search' }));

		expect(page.url.searchParams.has('q')).toBe(false);
		expect(page.url.searchParams.has('hit')).toBe(false);
		expect(searches).toHaveLength(1);
		expect(document.querySelectorAll('mark')).toHaveLength(0);
		expect(
			screen.getByRole('button', { name: 'Previous match' }),
		).toBeDisabled();
		expect(screen.getByRole('button', { name: 'Next match' })).toBeDisabled();
	});

	it('wraps previous and next navigation while replacing the URL', async () => {
		page.url = new URL(
			'http://localhost/collections/col1/documents/doc1?q=needle&hit=1',
		);
		mockSearch();
		renderPage({ partially_indexed: false });
		expect(await screen.findByText('1 of 3')).toBeVisible();

		await userEvent.click(screen.getByRole('button', { name: 'Next match' }));

		expect(await screen.findByText('2 of 3')).toBeVisible();
		expect(page.url.searchParams.get('hit')).toBe('2');
		expect(page.url.searchParams.get('q')).toBe('needle');

		await userEvent.click(
			screen.getByRole('button', { name: 'Previous match' }),
		);
		expect(await screen.findByText('1 of 3')).toBeVisible();

		await userEvent.click(
			screen.getByRole('button', { name: 'Previous match' }),
		);
		expect(await screen.findByText('3 of 3')).toBeVisible();
		expect(page.url.searchParams.get('hit')).toBe('3');
	});

	it('shows no matches and disables navigation', async () => {
		page.url = new URL(
			'http://localhost/collections/col1/documents/doc1?q=missing',
		);
		mockSearch(() => ({ partially_indexed: false, matches: [] }));
		renderPage({ partially_indexed: false });

		expect(await screen.findByText('No matches')).toBeVisible();
		expect(
			screen.getByRole('button', { name: 'Previous match' }),
		).toBeDisabled();
		expect(screen.getByRole('button', { name: 'Next match' })).toBeDisabled();
	});

	it('shows a deep-linked search error while manual search is disabled', async () => {
		page.url = new URL(
			'http://localhost/collections/col1/documents/doc1?q=needle',
		);
		const searches = mockSearch(() => {
			throw { code: 'not_indexed', message: 'Document is not indexed' };
		});
		renderPage({ partially_indexed: null });

		expect(await screen.findByRole('alert')).toHaveTextContent(
			'Document is not indexed',
		);
		expect(searches).toHaveLength(1);
		expect(
			screen.getByRole('textbox', { name: 'Search document' }),
		).toBeDisabled();
	});

	it('selects the first match when the requested hit is out of range', async () => {
		page.url = new URL(
			'http://localhost/collections/col1/documents/doc1?q=needle&hit=9',
		);
		mockSearch();
		renderPage({ partially_indexed: false });

		expect(await screen.findByText('1 of 3')).toBeVisible();
		const marks = document.querySelectorAll('mark');
		expect(marks[0]).toHaveAttribute('data-active', 'true');
	});
});
