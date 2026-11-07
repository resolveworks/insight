import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
import { page } from '$app/state';
import Page from './+page.svelte';

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
	mockIPC((cmd) => {
		if (cmd === 'get_pipeline_progress') return [];
	});
});

function renderPage(documents = [document]) {
	page.url = new URL('http://localhost/collections/col1/documents');
	page.params = { collectionId: 'col1' };
	render(Page, {
		props: {
			params: { collectionId: 'col1' },
			data: { collections: [collection], collection, documents },
		},
	});
}

describe('Collection documents page', () => {
	it('explains the empty collection and offers importing a PDF', () => {
		renderPage([]);

		expect(screen.getByText('No documents yet')).toBeInTheDocument();
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
		renderPage();

		const link = screen.getByRole('link', { name: /report\.pdf/ });
		expect(link).toHaveAttribute('href', '/collections/col1/documents/doc1');
		expect(screen.getByText('12 pages')).toBeInTheDocument();

		const del = screen.getByRole('button', { name: 'Delete' });
		expect(del.closest('a')).toBeNull();
		expect(screen.getAllByRole('button', { name: 'Import PDF' })).toHaveLength(
			1,
		);

		await fireEvent.click(del);
		expect(
			screen.getByRole('dialog', { name: 'Delete document' }),
		).toHaveTextContent('from this collection');
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
