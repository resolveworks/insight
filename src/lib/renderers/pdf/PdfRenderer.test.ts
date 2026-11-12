import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, waitFor, cleanup } from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
import PdfRenderer from './PdfRenderer.svelte';

/** A source read that never settles, so the viewer never mounts and no PDF engine runs. */
function pendingSource() {
	return new Promise<ArrayBuffer>(() => {});
}

afterEach(() => {
	cleanup();
	vi.restoreAllMocks();
});

describe('PdfRenderer', () => {
	it('reads the source of the document it is given', async () => {
		const calls: unknown[] = [];
		mockIPC((command, payload) => {
			if (command === 'get_document_source') {
				calls.push(payload);
				return pendingSource();
			}
		});

		render(PdfRenderer, {
			props: { collectionId: 'col1', documentId: 'doc1', page: null },
		});

		await waitFor(() =>
			expect(calls).toEqual([{ collectionId: 'col1', documentId: 'doc1' }]),
		);
	});

	it('shows a loading state while the source is read', async () => {
		mockIPC((command) => {
			if (command === 'get_document_source') return pendingSource();
		});

		render(PdfRenderer, {
			props: { collectionId: 'col1', documentId: 'doc1', page: null },
		});

		expect(
			await screen.findByRole('status', { name: 'Loading document' }),
		).toBeVisible();
	});

	it('shows an error when the source cannot be read', async () => {
		mockIPC((command) => {
			if (command === 'get_document_source') {
				throw { code: 'not_found', message: 'Document source not found' };
			}
		});

		render(PdfRenderer, {
			props: { collectionId: 'col1', documentId: 'doc1', page: null },
		});

		expect(await screen.findByRole('alert')).toHaveTextContent(
			'Document source not found',
		);
	});
});
