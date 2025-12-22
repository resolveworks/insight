// Mock for $app/stores
import { writable } from 'svelte/store';

export const page = writable({
	url: new URL('http://localhost/search'),
	params: {},
	route: { id: '/search' },
	status: 200,
	error: null,
	data: {},
	state: {},
	form: null,
});

export const navigating = writable(null);
export const updated = {
	subscribe: writable(false).subscribe,
	check: async () => false,
};
