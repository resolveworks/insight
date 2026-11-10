import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
import { load as documentLoad } from './+page';
import DocumentPage from './+page.svelte';
import type { PageProps } from './$types';

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

function renderPage() {
	render(DocumentPage, {
		props: {
			data: {
				collections: [collection],
				collection,
				documents: [document],
				document,
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
});
