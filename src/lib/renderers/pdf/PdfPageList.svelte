<script lang="ts">
	import { tick } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import { PageView } from '@embedpdf/svelte/page-view';
	import { RenderLayer } from '@embedpdf/svelte/render';
	import { usePageList, type PageInfo } from '@embedpdf/svelte/runtime';

	/** The gap between pages, as a page-flow document reads. */
	const PAGE_GAP = 16;
	/** One CSS pixel per 1/96 inch, one PDF point per 1/72 inch: 100% zoom. */
	const CSS_PIXELS_PER_POINT = 96 / 72;

	let { documentId, page }: { documentId: string; page: number | null } =
		$props();

	const pages = usePageList();
	const dpr = window.devicePixelRatio || 1;

	let container = $state<HTMLDivElement | null>(null);
	let width = $state(0);
	/** Indices of the pages close enough to the viewport to stay mounted. */
	const mounted = new SvelteSet<number>();
	let scrolledTo = $state<number | null>(null);

	/** The page at its natural 100% size, or the column's width when that is narrower. */
	function pageWidth(info: PageInfo): number {
		return Math.min(
			width,
			Math.floor(info.size.width * CSS_PIXELS_PER_POINT * info.userUnit),
		);
	}

	/** The page's footprint height, computed exactly as `PageView` sizes its box. */
	function pageHeight(info: PageInfo): number {
		const deviceWidth = Math.max(1, Math.round(pageWidth(info) * dpr));
		const deviceHeight = Math.max(
			1,
			Math.round((deviceWidth * info.size.height) / info.size.width),
		);
		return info.rotation % 180 === 0 ? deviceHeight / dpr : deviceWidth / dpr;
	}

	function scrollContainerOf(element: HTMLElement): HTMLElement | null {
		for (let node = element.parentElement; node; node = node.parentElement) {
			const overflowY = getComputedStyle(node).overflowY;
			if (overflowY === 'auto' || overflowY === 'scroll') return node;
		}
		return null;
	}

	$effect(() => {
		const element = container;
		if (!element) return;
		const measure = () => (width = element.clientWidth);
		measure();
		const observer = new ResizeObserver(measure);
		observer.observe(element);
		return () => observer.disconnect();
	});

	// The whole document is one placeholder per page; the ones near the scroller are mounted.
	$effect(() => {
		const element = container;
		const pageCount = pages.current.length;
		if (!element || width === 0 || pageCount === 0) return;
		let observer: IntersectionObserver | null = null;
		let cancelled = false;
		// Wait for this width's wrappers to be in the DOM before observing them.
		void tick().then(() => {
			if (cancelled || !container) return;
			observer = new IntersectionObserver(
				(entries) => {
					for (const entry of entries) {
						const index = Number(
							(entry.target as HTMLElement).dataset.pageIndex,
						);
						if (entry.isIntersecting) mounted.add(index);
						else mounted.delete(index);
					}
				},
				{ root: scrollContainerOf(container), rootMargin: '100% 0px' },
			);
			for (const node of container.querySelectorAll<HTMLElement>(
				'[data-page-index]',
			)) {
				observer.observe(node);
			}
		});
		return () => {
			cancelled = true;
			observer?.disconnect();
		};
	});

	// A page asked for by the URL lands at the top of the page's own scroll.
	$effect(() => {
		const target = page;
		const list = pages.current;
		if (target === null) {
			scrolledTo = null;
			return;
		}
		if (width === 0 || target < 1 || target > list.length) return;
		if (scrolledTo === target) return;
		const index = target - 1;
		void tick().then(() => {
			const wrapper = container?.querySelector<HTMLElement>(
				`[data-page-index="${index}"]`,
			);
			if (!wrapper) return;
			scrolledTo = target;
			wrapper.scrollIntoView({ block: 'start' });
		});
	});
</script>

<div
	bind:this={container}
	class="flex w-full flex-col items-center"
	style="gap: {PAGE_GAP}px;"
>
	{#if width > 0}
		{#each pages.current as info (info.ref.objectNumber)}
			<div data-page-index={info.index} style="height: {pageHeight(info)}px;">
				{#if mounted.has(info.index)}
					<PageView page={info.ref} {documentId} width={pageWidth(info)}>
						<RenderLayer />
					</PageView>
				{/if}
			</div>
		{/each}
	{/if}
</div>
