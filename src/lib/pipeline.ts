import type { PipelineProgress, StageProgress } from '#lib/types/documents.js';

/** Whether a stage still has work queued or running. */
export function stageIsActive(stage: StageProgress): boolean {
	return stage.pending > 0 || stage.active > 0;
}

/** Whether any pipeline stage for a collection is still working. */
export function pipelineIsActive(progress: PipelineProgress): boolean {
	return (
		stageIsActive(progress.store) ||
		stageIsActive(progress.extract) ||
		stageIsActive(progress.index)
	);
}
