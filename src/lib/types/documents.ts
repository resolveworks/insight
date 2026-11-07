/** A collection as returned by the command layer. */
export interface Collection {
	id: string;
	name: string;
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
	tags: string[];
	created_at: string;
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
