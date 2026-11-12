import { describe, it, expect } from 'vitest';
import { rendererFor } from './index.js';
import { pdfRenderer } from './pdf/index.js';

describe('rendererFor', () => {
	it('returns the PDF renderer for PDF documents', () => {
		expect(rendererFor('application/pdf')).toBe(pdfRenderer);
	});

	it('returns undefined for MIME types without a renderer', () => {
		expect(rendererFor('text/plain')).toBeUndefined();
	});
});
