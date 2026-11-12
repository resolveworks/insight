import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, waitFor, cleanup } from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
import type { Viewer } from '@embedpdf/viewer-svelte';
import PdfRenderer from './PdfRenderer.svelte';

vi.mock('@embedpdf/viewer', () => ({}));

interface ViewerElement extends HTMLElement {
	src?: Uint8Array;
	config: { onReady: (viewer: Viewer) => void };
}

const textEncoder = new TextEncoder();

function viewerElement(): ViewerElement {
	const element = document.querySelector('embedpdf-viewer');
	if (!element) throw new Error('PDF viewer is not mounted');
	return element as ViewerElement;
}

function makeViewer() {
	let opened: (() => void) | undefined;
	const goToPage = vi.fn();
	const viewer = {
		documents: {
			onOpened: vi.fn((callback: () => void) => {
				opened = callback;
			}),
		},
		get: vi.fn(() => ({ goToPage })),
	} as unknown as Viewer;

	return {
		viewer,
		goToPage,
		open() {
			if (!opened) throw new Error('No document-opened listener registered');
			opened();
		},
	};
}

afterEach(() => {
	cleanup();
	vi.restoreAllMocks();
});

describe('PdfRenderer', () => {
	it('opens the document source in the default viewer', async () => {
		const source = textEncoder.encode('%PDF-1.4');
		mockIPC((command) => {
			if (command === 'get_document_source') return source.buffer;
		});

		render(PdfRenderer, {
			props: { collectionId: 'col1', documentId: 'doc1', page: null },
		});

		await waitFor(() => expect(viewerElement().src).toBeInstanceOf(Uint8Array));
		expect(Array.from(viewerElement().src!)).toEqual(Array.from(source));
	});

	it('opens the requested page and follows page changes', async () => {
		const source = textEncoder.encode('%PDF-1.4');
		mockIPC((command) => {
			if (command === 'get_document_source') return source.buffer;
		});
		const { viewer, goToPage, open } = makeViewer();

		const { rerender } = render(PdfRenderer, {
			props: { collectionId: 'col1', documentId: 'doc1', page: 2 },
		});
		await waitFor(() => expect(viewerElement().src).toBeDefined());
		viewerElement().config.onReady(viewer);
		open();

		await waitFor(() => expect(goToPage).toHaveBeenCalledWith(1));

		await rerender({ collectionId: 'col1', documentId: 'doc1', page: 3 });
		await waitFor(() => expect(goToPage).toHaveBeenCalledWith(2));
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
