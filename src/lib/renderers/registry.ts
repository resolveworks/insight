import type { Component } from 'svelte';

export interface RendererProps {
	collectionId: string;
	documentId: string;
	page: number | null;
}

export interface Renderer {
	mimes: readonly string[];
	load(): Promise<Component<RendererProps>>;
}

export function buildRendererIndex(
	renderers: readonly Renderer[],
): ReadonlyMap<string, Renderer> {
	const index = new Map<string, Renderer>();
	for (const renderer of renderers) {
		for (const mime of renderer.mimes) {
			if (index.has(mime)) {
				throw new Error(`Multiple renderers claim MIME type "${mime}"`);
			}
			index.set(mime, renderer);
		}
	}
	return index;
}
