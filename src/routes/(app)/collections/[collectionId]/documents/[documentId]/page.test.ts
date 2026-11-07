import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
import { page } from '$app/state';
import DocumentPage from './+page.svelte';

vi.mock('@tauri-apps/api/event', () => ({
	listen: vi.fn(async () => () => {}),
}));

const document = {
	id: 'doc1',
	name: 'report.pdf',
	file_type: 'application/pdf',
	page_count: 12,
	tags: [],
	created_at: '2024-01-15T10:30:00Z',
};

describe('Document detail page', () => {
	it('names the document with a second-level heading, below the collection shell', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document') return document;
			if (cmd === 'get_document_text') return 'Extracted text';
		});
		page.url = new URL('http://localhost/collections/col1/documents/doc1');
		page.params = { collectionId: 'col1', documentId: 'doc1' };

		render(DocumentPage);

		const heading = await screen.findByRole('heading', {
			level: 2,
			name: 'report.pdf',
		});
		expect(heading).toBeInTheDocument();
		expect(screen.queryByRole('heading', { level: 1 })).not.toBeInTheDocument();
	});

	it('offers a single compact back link to the document list', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_document') return document;
			if (cmd === 'get_document_text') return 'Extracted text';
		});
		page.url = new URL('http://localhost/collections/col1/documents/doc1');
		page.params = { collectionId: 'col1', documentId: 'doc1' };

		render(DocumentPage);

		const backLinks = await screen.findAllByRole('link', {
			name: 'Back to documents',
		});
		expect(backLinks).toHaveLength(1);
		expect(backLinks[0]).toHaveAttribute('href', '/collections/col1/documents');
	});
});
