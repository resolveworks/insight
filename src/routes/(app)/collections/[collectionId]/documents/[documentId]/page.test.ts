import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
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

const document: Document = {
	id: 'doc1',
	name: 'report.pdf',
	file_type: 'application/pdf',
	page_count: 12,
	partially_indexed: null,
	tags: [],
	created_at: '2024-01-15T10:30:00Z',
};

function renderPage(overrides: Partial<typeof document> = {}) {
	const pageDocument = { ...document, ...overrides };
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

describe('Document detail load', () => {
	it('registers its dependency and returns the document', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document') return document;
		});
		const depends = vi.fn();

		const data = (await documentLoad({
			params: { collectionId: 'col1', documentId: 'doc1' },
			depends,
		} as unknown as Parameters<typeof documentLoad>[0])) as {
			document: typeof document;
		};

		expect(depends).toHaveBeenCalledWith('app:collection:col1');
		expect(data.document).toEqual(document);
	});
});

describe('Document detail page', () => {
	it('names the document with a content-level heading below the collection shell', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document_text') return 'Extracted text';
		});

		renderPage();

		expect(
			await screen.findByRole('heading', { level: 1, name: 'report.pdf' }),
		).toBeInTheDocument();
	});

	it('offers a single compact back link to the document list', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document_text') return 'Extracted text';
		});

		renderPage();

		const backLinks = await screen.findAllByRole('link', {
			name: 'Back to documents',
		});
		expect(backLinks).toHaveLength(1);
		expect(backLinks[0]).toHaveAttribute('href', '/collections/col1/documents');
	});

	it('summarises the document under its title', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document_text') return 'Extracted text';
		});

		renderPage({ page_count: 1, tags: ['budget', 'eu'] });

		expect(await screen.findByText('1 page')).toBeVisible();
		expect(
			screen.getByText(`Added ${formatTimestamp(document.created_at)}`),
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
