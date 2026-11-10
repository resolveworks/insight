<script lang="ts">
	import CodeBlock from './CodeBlock.svelte';
	import Disclosure from './Disclosure.svelte';
	import Icon from './Icon.svelte';
	import LoadingSpinner from './LoadingSpinner.svelte';
	import { formatResult, type ActivityStep } from '#lib/chat/activity.js';

	type Props = { step: ActivityStep };

	let { step }: Props = $props();

	const isError = $derived(step.kind === 'tool' && step.isError);
</script>

{#if step.kind === 'reasoning'}
	<Disclosure>
		{#snippet summary()}
			{#if step.streaming || step.text.trim() === ''}
				<LoadingSpinner size="xs" ariaLabel="Thinking" />
			{/if}
			<span class="shrink-0 font-medium text-neutral-800">Thought</span>
			{#if !step.streaming && step.text.trim() !== ''}
				<span class="min-w-0 truncate text-neutral-400">{step.preview}</span>
			{/if}
		{/snippet}
		<div class="pb-2 pl-6 text-sm leading-relaxed text-neutral-700">
			<pre class="whitespace-pre-wrap break-words">{step.text}</pre>
		</div>
	</Disclosure>
{:else}
	<Disclosure open={step.isError}>
		{#snippet summary()}
			{#if step.executing}
				<LoadingSpinner size="xs" ariaLabel="Running" />
			{:else if step.isError}
				<Icon name="warning" size="xs" class="text-red-600" label="Failed" />
			{:else}
				<Icon name="check" size="xs" class="text-green-600" label="Finished" />
			{/if}
			<span class="min-w-0 truncate">{step.label}</span>
			{#if step.hint}
				<span class="ml-auto shrink-0 pl-2 text-neutral-400">{step.hint}</span>
			{/if}
		{/snippet}

		<div class="space-y-2 pb-2 pl-6 pt-1">
			{#if step.argsLines.length > 0}
				<dl class="space-y-1 font-mono text-xs leading-relaxed">
					{#each step.argsLines as line (line.key)}
						<div class="flex gap-2">
							<dt class="shrink-0 text-neutral-400">{line.key}:</dt>
							<dd
								class="min-w-0 whitespace-pre-wrap break-words text-neutral-700"
							>
								{line.value}
							</dd>
						</div>
					{/each}
				</dl>
				{#if step.argsTruncated}
					<CodeBlock
						text={step.argsText}
						tone={isError ? 'error' : 'default'}
					/>
				{/if}
			{:else if !step.argsParseable && step.argsText.trim()}
				<CodeBlock text={step.argsText} tone={isError ? 'error' : 'default'} />
			{/if}

			{#if step.resultText !== null}
				<CodeBlock
					text={formatResult(step.resultText)}
					tone={isError ? 'error' : 'default'}
				/>
			{/if}
		</div>
	</Disclosure>
{/if}
