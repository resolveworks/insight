import type { Renderer } from '../registry.js';

export const pdfRenderer: Renderer = {
	mimes: ['application/pdf'],
	load: async () => (await import('./PdfRenderer.svelte')).default,
};
