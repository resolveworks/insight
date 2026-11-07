import { describe, it, expect, beforeEach, vi } from 'vitest';
import { invalidate } from '$app/navigation';
import { startInvalidationListeners } from './invalidation';

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

const activeProgress = {
	collection_id: 'col1',
	store: { pending: 0, active: 0, completed: 1, failed: 0 },
	extract: { pending: 1, active: 0, completed: 0, failed: 0 },
	index: { pending: 0, active: 0, completed: 0, failed: 0 },
};

const idleProgress = {
	collection_id: 'col1',
	store: { pending: 0, active: 0, completed: 1, failed: 0 },
	extract: { pending: 0, active: 0, completed: 1, failed: 0 },
	index: { pending: 0, active: 0, completed: 1, failed: 0 },
};

beforeEach(() => {
	listeners.clear();
});

describe('Backend invalidation signals', () => {
	it('re-reads a collection and the collection list after a peer change', async () => {
		await startInvalidationListeners();

		emit('collection-invalidated', { collection_id: 'col1' });

		expect(invalidate).toHaveBeenCalledWith('app:collections');
		expect(invalidate).toHaveBeenCalledWith('app:collection:col1');
	});

	it('does not re-read while a collection pipeline is still working', async () => {
		await startInvalidationListeners();

		emit('pipeline-progress', activeProgress);

		expect(invalidate).not.toHaveBeenCalled();
	});

	it('re-reads a collection when its pipeline goes idle', async () => {
		await startInvalidationListeners();

		emit('pipeline-progress', idleProgress);

		expect(invalidate).toHaveBeenCalledWith('app:collections');
		expect(invalidate).toHaveBeenCalledWith('app:collection:col1');
	});
});
