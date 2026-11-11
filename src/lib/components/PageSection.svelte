<script lang="ts">
	import type { Snippet } from 'svelte';
	import SectionHeading from './SectionHeading.svelte';

	interface Props {
		title: string;
		id?: string;
		children: Snippet;
	}

	let { title, id, children }: Props = $props();
</script>

<section>
	<div class="mb-3">
		<SectionHeading {title} {id} />
	</div>
	{@render children()}
</section>
