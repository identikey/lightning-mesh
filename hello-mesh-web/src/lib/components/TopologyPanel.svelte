<!--
	Live mesh topology panel (bead 4le). Renders the radio-layer graph — which
	nodes hear each other, at what expected throughput, and which links HWMP
	actually uses vs. hears-but-doesn't-use — as a static circular SVG layout.
	No chart libs; positions are computed once per node count.

	Data: directoryStore gives node identity (already polled by the page);
	topologyStore layers in /api/radio telemetry (self same-origin, neighbors
	cross-origin) every 10s and builds the graph. A node with no /api/radio
	(404, timeout, or pre-upgrade firmware) still appears from directory data,
	styled as "no telemetry".

	Preview without a live mesh: append `?mockTopology=1` to the page URL to
	render the hardcoded 4-node fixture instead of the live store.
-->
<script lang="ts">
	import { browser } from '$app/environment';
	import { Waypoints } from '@lucide/svelte';
	import { topologyStore, startTopologyPolling } from '$lib/topology/store.svelte';
	import { edgeStrength, type TopoEdge, type TopoGraph, type TopoNode } from '$lib/topology/graph';
	import { mockTopologyGraph } from '$lib/topology/fixtures';

	const useMock =
		browser && new URLSearchParams(window.location.search).get('mockTopology') === '1';

	if (!useMock) {
		$effect(startTopologyPolling);
	}

	const graph = $derived<TopoGraph | undefined>(useMock ? mockTopologyGraph : topologyStore.graph);
	const loaded = $derived(useMock || topologyStore.loaded);
	const lastUpdated = $derived(useMock ? Date.now() : topologyStore.lastUpdated);

	const WIDTH = 560;
	const HEIGHT = 360;
	const CENTER_X = WIDTH / 2;
	const CENTER_Y = HEIGHT / 2;
	const RADIUS = 130;

	interface Placed {
		node: TopoNode;
		x: number;
		y: number;
	}

	function layout(nodes: TopoNode[]): Placed[] {
		const n = nodes.length;
		if (n === 1) return [{ node: nodes[0], x: CENTER_X, y: CENTER_Y }];
		return nodes.map((node, i) => {
			const angle = (2 * Math.PI * i) / n - Math.PI / 2;
			return {
				node,
				x: CENTER_X + RADIUS * Math.cos(angle),
				y: CENTER_Y + RADIUS * Math.sin(angle)
			};
		});
	}

	const placed = $derived(graph ? layout(graph.nodes) : []);
	const placedByKey = $derived(new Map(placed.map((p) => [p.node.key, p])));

	interface RenderEdge {
		id: string;
		x1: number;
		y1: number;
		x2: number;
		y2: number;
		mx: number;
		my: number;
		strength: 'strong' | 'relay' | 'weak';
		label: string;
	}

	function formatLabel(edge: TopoEdge) {
		const a = edge.aToB ? Math.round(edge.aToB.throughputMbps) : undefined;
		const b = edge.bToA ? Math.round(edge.bToA.throughputMbps) : undefined;
		if (a !== undefined && b !== undefined) return `${a} ↔ ${b} Mbps`;
		if (a !== undefined) return `${a} Mbps →`;
		if (b !== undefined) return `← ${b} Mbps`;
		return 'unknown';
	}

	const renderEdges = $derived<RenderEdge[]>(
		(graph?.edges ?? [])
			.map((edge) => {
				const pa = placedByKey.get(edge.a);
				const pb = placedByKey.get(edge.b);
				if (!pa || !pb) return undefined;
				return {
					id: `${edge.a}|${edge.b}`,
					x1: pa.x,
					y1: pa.y,
					x2: pb.x,
					y2: pb.y,
					mx: (pa.x + pb.x) / 2,
					my: (pa.y + pb.y) / 2,
					strength: edgeStrength(edge),
					label: formatLabel(edge)
				};
			})
			.filter((e): e is RenderEdge => !!e)
	);

	function strokeVar(strength: 'strong' | 'relay' | 'weak'): string {
		if (strength === 'strong') return 'var(--success)';
		if (strength === 'relay') return 'var(--warning)';
		return 'var(--destructive)';
	}

	function strokeWidth(strength: 'strong' | 'relay' | 'weak'): number {
		return strength === 'strong' ? 3.5 : strength === 'relay' ? 2.5 : 1.5;
	}

	function lastUpdatedLabel(ts: number | undefined): string {
		if (!ts) return '';
		const secs = Math.round((Date.now() - ts) / 1000);
		if (secs < 2) return 'just now';
		if (secs < 60) return `${secs}s ago`;
		return `${Math.round(secs / 60)}m ago`;
	}
</script>

<section
	class="flex flex-col gap-4 rounded-lg border border-border bg-card p-4 text-card-foreground"
>
	<header class="flex flex-wrap items-center justify-between gap-2">
		<div class="flex items-center gap-2">
			<Waypoints class="size-4 text-muted-foreground" aria-hidden="true" />
			<h2 class="text-lg font-semibold">Mesh topology</h2>
		</div>
		{#if loaded && lastUpdated}
			<span class="text-xs text-muted-foreground">updated {lastUpdatedLabel(lastUpdated)}</span>
		{/if}
	</header>

	{#if !loaded}
		<div class="flex flex-col gap-3">
			<div class="h-[240px] w-full animate-pulse rounded-md bg-muted"></div>
			<p class="text-sm text-muted-foreground">Loading radio topology…</p>
		</div>
	{:else if !graph || graph.nodes.length === 0}
		<p class="text-sm text-muted-foreground">No mesh nodes known yet.</p>
	{:else}
		<div class="overflow-x-auto">
			<svg
				viewBox="0 0 {WIDTH} {HEIGHT}"
				width={WIDTH}
				height={HEIGHT}
				role="img"
				aria-label="Mesh radio topology graph"
				class="min-w-[420px] font-mono"
			>
				{#each renderEdges as edge (edge.id)}
					<line
						x1={edge.x1}
						y1={edge.y1}
						x2={edge.x2}
						y2={edge.y2}
						stroke={strokeVar(edge.strength)}
						stroke-width={strokeWidth(edge.strength)}
						stroke-dasharray={edge.strength === 'weak' ? '5 5' : undefined}
						opacity={edge.strength === 'weak' ? 0.6 : 0.85}
					/>
					<text
						x={edge.mx}
						y={edge.my}
						text-anchor="middle"
						fill={strokeVar(edge.strength)}
						font-size="10.5"
						class="pointer-events-none"
					>
						{edge.label}
					</text>
				{/each}

				{#each placed as p (p.node.key)}
					{@const reachable = !!p.node.radio}
					<g>
						<circle
							cx={p.x}
							cy={p.y}
							r={p.node.isSelf ? 30 : 26}
							fill="var(--card)"
							stroke={reachable
								? p.node.isSelf
									? 'var(--primary)'
									: 'var(--muted-foreground)'
								: 'var(--border)'}
							stroke-width={p.node.isSelf ? 3 : 2}
							stroke-dasharray={reachable ? undefined : '3 3'}
						/>
						<text
							x={p.x}
							y={p.y - 2}
							text-anchor="middle"
							fill="var(--foreground)"
							font-size="12"
							font-weight="600"
						>
							{p.node.label}
						</text>
						<text
							x={p.x}
							y={p.y + 12}
							text-anchor="middle"
							fill="var(--muted-foreground)"
							font-size="9.5"
						>
							{p.node.isSelf ? 'you are here' : reachable ? '' : 'no telemetry'}
						</text>
					</g>
				{/each}
			</svg>
		</div>

		<div class="flex flex-wrap gap-x-5 gap-y-1 font-mono text-[11px] text-muted-foreground">
			<span class="flex items-center gap-1.5">
				<span class="inline-block h-1 w-5 rounded-full" style="background: var(--success)"></span>
				strong link
			</span>
			<span class="flex items-center gap-1.5">
				<span class="inline-block h-1 w-5 rounded-full" style="background: var(--warning)"></span>
				relay / moderate link
			</span>
			<span class="flex items-center gap-1.5">
				<span
					class="inline-block h-1 w-5 rounded-full opacity-60"
					style="background: var(--destructive)"
				></span>
				weak / unused link
			</span>
			<span class="flex items-center gap-1.5">
				<span class="inline-block h-3 w-3 rounded-full border border-dashed border-border"></span>
				no telemetry
			</span>
		</div>
	{/if}
</section>
