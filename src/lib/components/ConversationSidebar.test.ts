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

	it('invokes delete_conversation when the delete button is clicked', async () => {
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
			const deleteCall = calls.find((c) => c.cmd === 'delete_conversation');
			expect(deleteCall?.args).toEqual({ conversationId: 'c1' });
		});
	});
});
