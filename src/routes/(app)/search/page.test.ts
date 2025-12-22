import { describe, it, expect, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { mockIPC } from '@tauri-apps/api/mocks';
import SearchPage from './+page.svelte';

const mockCollections = [
	{ id: 'col1', name: 'Research', document_count: 5 },
	{ id: 'col2', name: 'News', document_count: 3 },
];

const mockSearchResults = {
	hits: [
		{
			chunk_id: 'doc1_0',
			document: {
				id: 'doc1',
				name: 'Climate Report.pdf',
				pdf_hash: '',
				text_hash: '',
				page_count: 10,
				tags: [],
				created_at: '2024-01-01',
			},
			collection_id: 'col1',
			snippet: 'Climate change is affecting global temperatures...',
			score: 0.95,
		},
		{
			chunk_id: 'doc2_1',
			document: {
				id: 'doc2',
				name: 'Weather Data.pdf',
				pdf_hash: '',
				text_hash: '',
				page_count: 5,
				tags: [],
				created_at: '2024-01-02',
			},
			collection_id: 'col1',
			snippet: 'Weather patterns show significant changes...',
			score: 0.85,
		},
	],
	total_hits: 2,
	page: 0,
	page_size: 20,
};

describe('Search Page', () => {
	beforeEach(() => {
		mockIPC((cmd, args) => {
			if (cmd === 'get_collections') {
				return mockCollections;
			}
			if (cmd === 'search') {
				const { query } = args as { query: string };
				if (!query || query.trim() === '') {
					return { hits: [], total_hits: 0, page: 0, page_size: 20 };
				}
				return mockSearchResults;
			}
			return null;
		});
	});

	it('renders search input and initial state', async () => {
		render(SearchPage);

		const searchInput = screen.getByPlaceholderText('Search documents...');
		expect(searchInput).toBeInTheDocument();
		expect(screen.getByText('Start typing to search')).toBeInTheDocument();
	});

	it('loads and displays collections in sidebar', async () => {
		render(SearchPage);

		await waitFor(() => {
			expect(screen.getByText('Research')).toBeInTheDocument();
			expect(screen.getByText('News')).toBeInTheDocument();
		});
	});

	it('displays search results when query is entered', async () => {
		const user = userEvent.setup();
		render(SearchPage);

		const searchInput = screen.getByPlaceholderText('Search documents...');
		await user.type(searchInput, 'climate');

		await waitFor(() => {
			expect(screen.getByText('2 results found')).toBeInTheDocument();
			expect(screen.getByText('Climate Report.pdf')).toBeInTheDocument();
			expect(screen.getByText('Weather Data.pdf')).toBeInTheDocument();
		});
	});

	it('displays snippets and scores for results', async () => {
		const user = userEvent.setup();
		render(SearchPage);

		const searchInput = screen.getByPlaceholderText('Search documents...');
		await user.type(searchInput, 'climate');

		await waitFor(() => {
			expect(
				screen.getByText('Climate change is affecting global temperatures...'),
			).toBeInTheDocument();
			expect(screen.getByText('Score: 0.95')).toBeInTheDocument();
		});
	});

	it('shows no results message when search returns empty', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_collections') return mockCollections;
			if (cmd === 'search') {
				return { hits: [], total_hits: 0, page: 0, page_size: 20 };
			}
			return null;
		});

		const user = userEvent.setup();
		render(SearchPage);

		const searchInput = screen.getByPlaceholderText('Search documents...');
		await user.type(searchInput, 'nonexistent');

		await waitFor(() => {
			expect(screen.getByText('No results found')).toBeInTheDocument();
		});
	});

	it('toggles collection filter', async () => {
		const user = userEvent.setup();
		render(SearchPage);

		await waitFor(() => {
			expect(screen.getByText('Research')).toBeInTheDocument();
		});

		const researchCheckbox = screen.getByRole('checkbox', {
			name: /Research/i,
		});
		await user.click(researchCheckbox);

		expect(researchCheckbox).toBeChecked();
	});

	it('clears collection filters', async () => {
		const user = userEvent.setup();
		render(SearchPage);

		await waitFor(() => {
			expect(screen.getByText('Research')).toBeInTheDocument();
		});

		// Select a filter
		const checkbox = screen.getByRole('checkbox', { name: /Research/i });
		await user.click(checkbox);

		// Wait for clear button to appear (indicates filter was selected)
		await waitFor(() => {
			expect(screen.getByText('Clear filters')).toBeInTheDocument();
		});

		const clearButton = screen.getByText('Clear filters');
		await user.click(clearButton);

		// Button should disappear after clearing
		await waitFor(() => {
			expect(screen.queryByText('Clear filters')).not.toBeInTheDocument();
		});
	});

	it('has semantic ratio slider', () => {
		render(SearchPage);

		expect(screen.getByText('Keyword')).toBeInTheDocument();
		expect(screen.getByText('Semantic')).toBeInTheDocument();

		const slider = screen.getByRole('slider');
		expect(slider).toBeInTheDocument();
		expect(slider).toHaveAttribute('min', '0');
		expect(slider).toHaveAttribute('max', '1');
	});

	it('shows pagination when results exceed page size', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_collections') return mockCollections;
			if (cmd === 'search') {
				return {
					hits: mockSearchResults.hits,
					total_hits: 50,
					page: 0,
					page_size: 20,
				};
			}
			return null;
		});

		const user = userEvent.setup();
		render(SearchPage);

		const searchInput = screen.getByPlaceholderText('Search documents...');
		await user.type(searchInput, 'climate');

		await waitFor(() => {
			expect(screen.getByText('Page 1 of 3')).toBeInTheDocument();
			expect(screen.getByText('Previous')).toBeInTheDocument();
			expect(screen.getByText('Next')).toBeInTheDocument();
		});
	});

	it('disables previous button on first page', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_collections') return mockCollections;
			if (cmd === 'search') {
				return { ...mockSearchResults, total_hits: 50 };
			}
			return null;
		});

		const user = userEvent.setup();
		render(SearchPage);

		await user.type(screen.getByPlaceholderText('Search documents...'), 'test');

		await waitFor(() => {
			const prevButton = screen.getByText('Previous');
			expect(prevButton).toBeDisabled();
		});
	});
});
