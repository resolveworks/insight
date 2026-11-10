<script lang="ts">
	import LoadingSpinner from './LoadingSpinner.svelte';
	import { formatResult, type ActivityStep } from '#lib/chat/activity.js';

	type Props = { step: ActivityStep };

	let { step }: Props = $props();
</script>

{#if step.kind === 'reasoning'}
	<!-- Static `open` is absent: collapsed until the reader expands it. -->
	<details class="group">
		<summary
			class="flex cursor-pointer list-none items-center gap-2 py-1 text-xs hover:text-neutral-800 [&::-webkit-details-marker]:hidden"
		>
			<span class="flex h-3.5 w-3.5 shrink-0 items-center justify-center">
				{#if step.streaming || step.text.trim() === ''}
					<LoadingSpinner size="sm" class="h-3.5 w-3.5" ariaLabel="Thinking" />
				{/if}
			</span>
			<span class="shrink-0 font-medium text-neutral-600">Thought</span>
			{#if !step.streaming && step.text.trim() !== ''}
				<span class="min-w-0 truncate text-subtle">{step.preview}</span>
			{/if}
		</summary>
		<div class="pb-2 pr-2 pl-5">
			<pre
				class="whitespace-pre-wrap break-words text-xs text-neutral-600">{step.text}</pre>
		</div>
	</details>
{:else}
	<details open={step.isError} class="group">
		<summary
			class="flex cursor-pointer list-none items-center gap-2 py-1 text-xs text-neutral-700 hover:text-neutral-900 [&::-webkit-details-marker]:hidden"
		>
			<span class="flex h-3.5 w-3.5 shrink-0 items-center justify-center">
				{#if step.executing}
					<LoadingSpinner size="sm" class="h-3.5 w-3.5" ariaLabel="Running" />
				{:else if step.isError}
					<svg
						class="h-3.5 w-3.5 text-error"
						viewBox="0 0 20 20"
						fill="currentColor"
						aria-label="Failed"
						role="img"
					>
						<path
							fill-rule="evenodd"
							d="M8.485 2.495c.673-1.167 2.357-1.167 3.03 0l6.28 10.875c.673 1.167-.17 2.625-1.516 2.625H3.72c-1.347 0-2.189-1.458-1.515-2.625L8.485 2.495ZM10 6a.75.75 0 0 1 .75.75v3.5a.75.75 0 0 1-1.5 0v-3.5A.75.75 0 0 1 10 6Zm0 9a1 1 0 1 0 0-2 1 1 0 0 0 0 2Z"
							clip-rule="evenodd"
						/>
					</svg>
				{:else}
					<svg
						class="h-3.5 w-3.5 text-success"
						viewBox="0 0 20 20"
						fill="currentColor"
						aria-label="Finished"
						role="img"
					>
						<path
							fill-rule="evenodd"
							d="M16.704 4.153a.75.75 0 0 1 .143 1.052l-8 10.5a.75.75 0 0 1-1.127.075l-4.5-4.5a.75.75 0 0 1 1.06-1.06l3.894 3.893 7.48-9.817a.75.75 0 0 1 1.05-.143Z"
							clip-rule="evenodd"
						/>
					</svg>
				{/if}
			</span>
			<span class="min-w-0 truncate">{step.label}</span>
			{#if step.hint}
				<span class="ml-auto shrink-0 pl-2 text-subtle">{step.hint}</span>
			{/if}
		</summary>

		<div class="space-y-2 pb-2 pr-2 pl-5 pt-1">
			{#if step.argsLines.length > 0}
				<dl class="space-y-1 font-mono text-[11px] leading-relaxed">
					{#each step.argsLines as line (line.key)}
						<div class="flex gap-2">
							<dt class="shrink-0 text-subtle">{line.key}:</dt>
							<dd
								class="min-w-0 whitespace-pre-wrap break-words text-neutral-600"
							>
								{line.value}
							</dd>
						</div>
					{/each}
				</dl>
				{#if step.argsTruncated}
					<pre
						class="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded bg-surface-dim p-2 font-mono text-[11px] leading-relaxed text-neutral-600">{step.argsText}</pre>
				{/if}
			{:else if !step.argsParseable && step.argsText.trim()}
				<pre
					class="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded bg-surface-dim p-2 font-mono text-[11px] leading-relaxed text-neutral-600">{step.argsText}</pre>
			{/if}

			{#if step.resultText !== null}
				<pre
					class="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded bg-surface-dim p-2 font-mono text-[11px] leading-relaxed {step.isError
						? 'text-error'
						: 'text-neutral-700'}">{formatResult(step.resultText)}</pre>
			{/if}
		</div>
	</details>
{/if}
