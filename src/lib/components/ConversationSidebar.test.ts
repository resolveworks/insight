import { describe, it, expect, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/svelte';
import ConversationSidebar from './ConversationSidebar.svelte';

const summary = (id: string, title: string) => ({ id, title });

const conversations = [
	summary('c1', 'Climate research'),
	summary('c2', 'Budget analysis'),
];

describe('ConversationSidebar', () => {
	it('shows the conversations it is given', () => {
		render(ConversationSidebar, {
			props: { conversations, activeId: 'c1' },
		});

		expect(screen.getByText('Climate research')).toBeInTheDocument();
		expect(screen.getByText('Budget analysis')).toBeInTheDocument();
		expect(
			screen.getByRole('button', { name: 'New Chat' }),
		).toBeInTheDocument();
	});

	it('marks the active conversation and keeps delete keyboard accessible', () => {
		render(ConversationSidebar, {
			props: { conversations, activeId: 'c1' },
		});

		const active = screen.getByRole('button', {
			name: 'Climate research',
		});
		expect(active).toHaveAttribute('aria-current', 'true');
		expect(
			screen.getByRole('button', { name: 'Budget analysis' }),
		).not.toHaveAttribute('aria-current');
		expect(screen.getAllByLabelText('Delete conversation')[0].tagName).toBe(
			'BUTTON',
		);
	});

	it('shows empty state when the list is empty', () => {
		render(ConversationSidebar, {
			props: { conversations: [], activeId: null },
		});

		expect(screen.getByText('No conversations yet')).toBeInTheDocument();
	});

	it('asks for confirmation before deleting a conversation', async () => {
		const onDelete = vi.fn();
		render(ConversationSidebar, {
			props: { conversations, activeId: 'c1', onDelete },
		});

		screen.getAllByLabelText('Delete conversation')[0].click();
		await waitFor(() => {
			expect(
				screen.getByRole('dialog', { name: 'Delete conversation' }),
			).toBeInTheDocument();
		});
		expect(
			screen.getByText(/This conversation's history will be removed\./),
		).toBeInTheDocument();
		expect(onDelete).not.toHaveBeenCalled();

		within(screen.getByRole('dialog'))
			.getByRole('button', { name: 'Delete conversation' })
			.click();

		expect(onDelete).toHaveBeenCalledWith('c1');
	});

	it('cancels deletion without asking the parent to delete', async () => {
		const onDelete = vi.fn();
		render(ConversationSidebar, {
			props: { conversations, activeId: 'c1', onDelete },
		});

		screen.getAllByLabelText('Delete conversation')[0].click();
		await waitFor(() => {
			expect(
				screen.getByRole('button', { name: 'Cancel' }),
			).toBeInTheDocument();
		});
		screen.getByRole('button', { name: 'Cancel' }).click();

		await waitFor(() => {
			expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
		});
		expect(onDelete).not.toHaveBeenCalled();
	});
});
