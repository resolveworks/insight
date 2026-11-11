<script lang="ts">
	interface Props {
		text: string;
	}

	let { text }: Props = $props();
</script>

<pre
	class="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-md border border-neutral-200 bg-neutral-100 p-3 font-mono text-xs leading-relaxed text-neutral-700">{text}</pre>
