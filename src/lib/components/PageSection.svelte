<script lang="ts">
	import type { Snippet } from 'svelte';
	import SectionHeading from './SectionHeading.svelte';

	interface Props {
		title: string;
		id?: string;
		divider?: boolean;
		children: Snippet;
	}

	let { title, id, divider = false, children }: Props = $props();
</script>

<section class={divider ? 'border-t border-line pt-6' : ''}>
	<div class="mb-3">
		<SectionHeading {title} {id} />
	</div>
	{@render children()}
</section>
