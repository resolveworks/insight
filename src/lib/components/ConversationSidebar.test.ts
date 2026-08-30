import { describe, it, expect, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import ConversationSidebar from './ConversationSidebar.svelte';

describe('ConversationSidebar', () => {
	beforeEach(() => {
		clearMocks();
	});

	it('shows the conversation list without a chat provider configured', async () => {
		mockIPC((cmd) => {
			if (cmd === 'list_conversations') {
				return [
					{
						id: 'c1',
						title: 'Climate research',
						updated_at: '2024-02-01T00:00:00Z',
					},
					{
						id: 'c2',
						title: 'Budget analysis',
						updated_at: '2024-01-15T00:00:00Z',
					},
				];
			}
		});

		render(ConversationSidebar, { props: {} });

		await waitFor(() => {
			expect(screen.getByText('Climate research')).toBeInTheDocument();
		});
		expect(screen.getByText('Budget analysis')).toBeInTheDocument();
		expect(screen.queryByText('Loading...')).not.toBeInTheDocument();
	});

	it('shows empty state when the list is empty', async () => {
		mockIPC((cmd) => {
			if (cmd === 'list_conversations') return [];
		});

		render(ConversationSidebar, { props: {} });

		await waitFor(() => {
			expect(screen.getByText('No conversations yet')).toBeInTheDocument();
		});
	});

	it('deletes a conversation after confirmation', async () => {
		const calls: Array<{ cmd: string; args: Record<string, unknown> }> = [];
		mockIPC((cmd, args) => {
			calls.push({ cmd, args: args as Record<string, unknown> });
			if (cmd === 'list_conversations') {
				return [
					{
					id: 'c1',
					title: 'Climate research',
					updated_at: '2024-02-01T00:00:00Z',
				},
				];
			}
			if (cmd === 'delete_conversation') return null;
		});

		render(ConversationSidebar, { props: {} });

		await waitFor(() => {
			expect(screen.getByText('Climate research')).toBeInTheDocument();
		});

		// Opening the confirm dialog alone does not delete anything.
		screen.getByLabelText('Delete chat').click();
		await waitFor(() => {
			expect(screen.getByText('Delete chat')).toBeInTheDocument();
		});
		expect(
			screen.getByText(
				/This conversation's history will be removed\./,
			),
		).toBeInTheDocument();
		expect(calls.find((c) => c.cmd === 'delete_conversation')).toBeUndefined();

		screen.getByRole('button', { name: 'Delete' }).click();

		await waitFor(() => {
			const deleteCall = calls.find((c) => c.cmd === 'delete_conversation');
			expect(deleteCall?.args).toEqual({ conversationId: 'c1' });
		});
	});

	it('cancels deletion without invoking the backend', async () => {
		const calls: Array<{ cmd: string; args: Record<string, unknown> }> = [];
		mockIPC((cmd, args) => {
			calls.push({ cmd, args: args as Record<string, unknown> });
			if (cmd === 'list_conversations') {
				return [
					{
						id: 'c1',
						title: 'Climate research',
						updated_at: '2024-02-01T00:00:00Z',
					},
				];
			}
			if (cmd === 'delete_conversation') return null;
		});

		render(ConversationSidebar, { props: {} });

		await waitFor(() => {
			expect(screen.getByText('Climate research')).toBeInTheDocument();
		});

		screen.getByLabelText('Delete chat').click();
		await waitFor(() => {
			expect(screen.getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
		});
		screen.getByRole('button', { name: 'Cancel' }).click();

		await waitFor(() => {
			expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
		});
		expect(calls.find((c) => c.cmd === 'delete_conversation')).toBeUndefined();
	});
});
