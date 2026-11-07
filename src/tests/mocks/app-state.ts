// Mock for $app/state
interface MockPage {
	url: URL;
	params: Record<string, string>;
	route: { id: string | null };
	status: number;
	error: { message: string } | null;
	data: Record<string, unknown>;
	state: Record<string, unknown>;
	form: null;
}

export const page: MockPage = {
	url: new URL('http://localhost/search'),
	params: {},
	route: { id: '/search' },
	status: 200,
	error: null,
	data: {},
	state: {},
	form: null,
};
