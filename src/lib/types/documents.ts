/** A collection as returned by the command layer. */
export interface Collection {
	id: string;
	/** `null` until the collection's metadata has synced from a peer. */
	name: string | null;
	document_count: number;
	total_pages: number | null;
	created_at?: string;
}

/** Document metadata as returned by the command layer. */
export interface Document {
	id: string;
	name: string;
	file_type: string;
	page_count: number | null;
	/**
	 * Whether the search index covers only part of the document.
	 * `null` while the document has no index record yet.
	 */
	partially_indexed: boolean | null;
	tags: string[];
	created_at: string;
}

/** A deleted document as returned by the command layer. */
export interface DeletedDocument {
	id: string;
	name: string;
	deleted_by: string;
	deleted_at: string;
}

/** An inclusive range of source page numbers. */
export interface SourceRange {
	start: number;
	end: number;
}

/** One occurrence of a searched phrase in a document's content. */
export interface DocumentSearchMatch {
	/** The exact matched substring. */
	content: string;
	/** Zero-based Unicode character offset of the match in the extraction. */
	offset: number;
	/** Unicode character offset one past the match. */
	next_offset: number;
	/** Inclusive source pages covered by the match, when the extraction maps pages. */
	source_range: SourceRange | null;
}

/** Every literal phrase match within one document. */
export interface DocumentSearchResults {
	/** Whether part of the document is outside the search index's coverage. */
	partially_indexed: boolean;
	/** Matches in content order. */
	matches: DocumentSearchMatch[];
}

/** Per-stage progress counts. */
export interface StageProgress {
	pending: number;
	active: number;
	completed: number;
	failed: number;
}

/** Pipeline progress for a collection across all stages. */
export interface PipelineProgress {
	collection_id: string;
	store: StageProgress;
	extract: StageProgress;
	index: StageProgress;
}

/** What an import stored, plus the files it could not store. */
export interface ImportOutcome {
	documents: Document[];
	failures: { path: string; message: string }[];
}
