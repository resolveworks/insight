<script lang="ts">
	import { untrack } from 'svelte';
	import { RenderLayer } from '@embedpdf/svelte/render';
	import {
		SearchLayer,
		useSearch,
		useSearchEvent,
	} from '@embedpdf/svelte/search';
	import { Stage, useStage } from '@embedpdf/svelte/stage';
	import { usePageList } from '@embedpdf/svelte/runtime';
	import { describeError } from '#lib/api/provider.js';
	import Alert from '#lib/components/Alert.svelte';

	interface Props {
		page: number | null;
		query: string;
		activeHit: number;
		visible: boolean;
	}

	let { page, query, activeHit, visible }: Props = $props();

	const search = useSearch();
	const stage = useStage();
	const pages = usePageList();

	let searchError = $state<string | null>(null);
	let lastQuery: string | null = null;
	let lastActiveHit = -1;
	let lastVisible = false;
	let request = 0;

	useSearchEvent(
		(search) => search.onFailed,
		(event) => {
			searchError = event.error.message;
		},
	);

	$effect(() => {
		const currentQuery = query;
		const currentHit = activeHit;
		const shown = visible;

		if (currentQuery !== lastQuery) {
			lastQuery = currentQuery;
			const id = ++request;
			lastActiveHit = -1;
			searchError = null;

			if (currentQuery.trim() === '') {
				search.clear();
			} else {
				void search
					.search({ text: currentQuery }, { from: 0 })
					.then((result) => {
						if (id !== request || result.status !== 'complete') return;
						const hit = activeHit;
						if (hit === lastActiveHit) return;
						lastActiveHit = hit;
						// The search makes its first hit active without moving the view.
						search.goToHit(hit, { behavior: 'instant' });
					})
					.catch((error: unknown) => {
						if (id !== request) return;
						searchError = describeError(error);
					});
			}
		} else if (currentQuery.trim() !== '' && currentHit !== lastActiveHit) {
			if (search.goToHit(currentHit) !== null) lastActiveHit = currentHit;
		}

		if (shown !== lastVisible) {
			lastVisible = shown;
			if (shown && currentQuery.trim() !== '') {
				const active = untrack(() => search.getActiveHit());
				if (active) search.revealActiveHit();
			}
		}
	});

	// A page from the URL lands at the top of its own scroll, unless a search hit drives the view.
	let navigatedPage: number | null = null;

	$effect(() => {
		const target = page;
		const count = pages.current.length;
		if (query.trim() !== '') {
			navigatedPage = null;
			return;
		}
		if (target === null) {
			navigatedPage = null;
			return;
		}
		if (target < 1 || target > count || navigatedPage === target) return;
		navigatedPage = target;
		stage.goToPage(target - 1, { behavior: 'instant' });
	});
</script>

{#if searchError}
	<Alert>{searchError}</Alert>
{/if}

<Stage
	class="h-[75vh] w-full rounded-lg border border-neutral-200 bg-neutral-100"
>
	<RenderLayer />
	<SearchLayer />
</Stage>
