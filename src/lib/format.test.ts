import { describe, it, expect } from 'vitest';
import { shortenId } from './format';

describe('shortenId', () => {
	it('leaves an id shorter than the limit unchanged', () => {
		expect(shortenId('abc')).toBe('abc');
	});

	it('leaves an id of exactly the limit unchanged', () => {
		expect(shortenId('abcdefgh')).toBe('abcdefgh');
	});

	it('truncates a longer id to the limit and appends an ellipsis', () => {
		expect(shortenId('abcdefghijklmnop')).toBe('abcdefgh…');
	});
});
