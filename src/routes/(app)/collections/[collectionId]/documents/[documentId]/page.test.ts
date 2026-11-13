import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, cleanup, fireEvent } from '@testing-library/svelte';
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

/**
 * A source read that never settles: the viewer only mounts once the source arrives, and a real
 * PDF engine has no business running in these tests.
 */
function pendingSource() {
	return new Promise<ArrayBuffer>(() => {});
}

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
			if (cmd === 'get_document_source') return pendingSource();
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
			if (cmd === 'get_document_source') return pendingSource();
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
			if (cmd === 'get_document_source') return pendingSource();
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
			if (cmd === 'get_document_source') return pendingSource();
		});

		renderPage();
		await userEvent.click(await screen.findByRole('tab', { name: 'Text' }));

		expect(await screen.findByText('Extracted text')).toBeVisible();
	});

	it('warns that search covers only part of a partially indexed document', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document_text') return 'Extracted text';
			if (cmd === 'get_document_source') return pendingSource();
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
			if (cmd === 'get_document_source') return pendingSource();
		});

		renderPage({ partially_indexed: false });
		await userEvent.click(await screen.findByRole('tab', { name: 'Text' }));

		expect(await screen.findByText('Extracted text')).toBeVisible();
		expect(screen.queryByText('Partially searchable')).not.toBeInTheDocument();
	});

	it('shows no partial-index warning while coverage is unknown', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document_text') return 'Extracted text';
			if (cmd === 'get_document_source') return pendingSource();
		});

		renderPage({ partially_indexed: null });
		await userEvent.click(await screen.findByRole('tab', { name: 'Text' }));

		expect(await screen.findByText('Extracted text')).toBeVisible();
		expect(screen.queryByText('Partially searchable')).not.toBeInTheDocument();
	});
});

describe('Document detail source and text tabs', () => {
	function tabPanel(id: 'source-panel' | 'text-panel') {
		const panel = document.getElementById(id);
		if (!panel) throw new Error(`Missing tab panel #${id}`);
		return panel;
	}

	it('shows source and text tabs with the source selected', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document_text') return 'Extracted text';
			if (cmd === 'get_document_source') return pendingSource();
		});

		renderPage();

		const sourceTab = await screen.findByRole('tab', { name: 'Source' });
		const textTab = screen.getByRole('tab', { name: 'Text' });
		expect(sourceTab).toHaveAttribute('aria-selected', 'true');
		expect(textTab).toHaveAttribute('aria-selected', 'false');
		expect(tabPanel('text-panel')).toHaveAttribute('hidden');
	});

	it('switches to the extraction and back to the source', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document_text') return 'Extracted text';
			if (cmd === 'get_document_source') return pendingSource();
		});

		renderPage();

		await userEvent.click(await screen.findByRole('tab', { name: 'Text' }));

		expect(screen.getByRole('tab', { name: 'Text' })).toHaveAttribute(
			'aria-selected',
			'true',
		);
		expect(await screen.findByText('Extracted text')).toBeVisible();
		expect(tabPanel('source-panel')).toHaveAttribute('hidden');

		await userEvent.click(screen.getByRole('tab', { name: 'Source' }));

		expect(screen.getByRole('tab', { name: 'Source' })).toHaveAttribute(
			'aria-selected',
			'true',
		);
		expect(tabPanel('source-panel')).not.toHaveAttribute('hidden');
		expect(tabPanel('text-panel')).toHaveAttribute('hidden');
	});

	it('renders text without tabs and never reads the source', async () => {
		const commands: string[] = [];
		mockIPC((cmd) => {
			commands.push(cmd);
			if (cmd === 'get_document_text') return 'Extracted text';
		});

		renderPage({ name: 'notes.txt', file_type: 'text/plain' });

		expect(await screen.findByText('Extracted text')).toBeVisible();
		expect(screen.queryByRole('tablist')).not.toBeInTheDocument();
		expect(commands).not.toContain('get_document_source');
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
			if (cmd === 'get_document_source') return pendingSource();
			if (cmd === 'search_document') {
				searches.push(payload);
				return handler(payload);
			}
		});
		return searches;
	}

	it('runs a deep-linked search immediately, selects its hit and opens Text', async () => {
		page.url = new URL(
			'http://localhost/collections/col1/documents/doc1?q=needle&hit=2&view=text',
		);
		const searches = mockSearch();

		renderPage({ partially_indexed: false });

		expect(await screen.findByText('2 of 3')).toBeVisible();
		expect(searches).toEqual([
			{ collectionId: 'col1', documentId: 'doc1', query: 'needle' },
		]);
		expect(screen.getByRole('tab', { name: 'Text' })).toHaveAttribute(
			'aria-selected',
			'true',
		);
		const marks = document.querySelectorAll('mark');
		expect(marks).toHaveLength(3);
		expect(marks[1]).toHaveAttribute('data-active', 'true');
	});

	it('keeps Source selected for a query deep link without a view', async () => {
		page.url = new URL(
			'http://localhost/collections/col1/documents/doc1?q=needle&hit=2',
		);
		mockSearch();

		renderPage({ partially_indexed: false });

		expect(await screen.findByText('2 of 3')).toBeVisible();
		expect(screen.getByRole('tab', { name: 'Source' })).toHaveAttribute(
			'aria-selected',
			'true',
		);
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
		expect(page.url.searchParams.has('page')).toBe(false);
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
			'http://localhost/collections/col1/documents/doc1?q=needle&hit=2&view=text',
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
			'http://localhost/collections/col1/documents/doc1?q=needle&hit=1&view=text&page=2',
		);
		mockSearch();
		renderPage({ partially_indexed: false });
		expect(await screen.findByText('1 of 3')).toBeVisible();

		await userEvent.click(screen.getByRole('button', { name: 'Next match' }));

		expect(await screen.findByText('2 of 3')).toBeVisible();
		expect(page.url.searchParams.get('hit')).toBe('2');
		expect(page.url.searchParams.get('q')).toBe('needle');
		expect(page.url.searchParams.get('view')).toBe('text');
		expect(page.url.searchParams.has('page')).toBe(false);

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
			'http://localhost/collections/col1/documents/doc1?q=missing&view=text',
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
			'http://localhost/collections/col1/documents/doc1?q=needle&hit=9&view=text',
		);
		mockSearch();
		renderPage({ partially_indexed: false });

		expect(await screen.findByText('1 of 3')).toBeVisible();
		const marks = document.querySelectorAll('mark');
		expect(marks[0]).toHaveAttribute('data-active', 'true');
	});
});
