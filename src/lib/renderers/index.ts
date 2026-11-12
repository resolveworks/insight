import { pdfRenderer } from './pdf/index.js';
import { buildRendererIndex, type Renderer } from './registry.js';

const index = buildRendererIndex([pdfRenderer]);

/** The renderer registered for a MIME type, if any. */
export function rendererFor(mime: string): Renderer | undefined {
	return index.get(mime);
}
