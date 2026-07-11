<!--
	Routers: the nodes that make up this mesh, and the radio links between them.
	Merges the old directory "Neighbors" list with the topology graph. Compact
	rows name each router (muted subnet/hex fallback when unnamed) and mark the one
	you're on; the SVG below shows which routers hear each other over the air.
	Rides the shared directory poll for identity and the topology store for radio
	telemetry. Preview without a live mesh: ?mockTopology=1 or ?mockDirectory=1.
-->
<script lang="ts">
	import { browser } from '$app/environment';
	import { directoryStore } from '$lib/directory/store.svelte';
	import { topologyStore, startTopologyPolling } from '$lib/topology/store.svelte';
	import { mockTopologyGraph } from '$lib/topology/fixtures';
	import type { TopoGraph } from '$lib/topology/graph';
	import RadioGraph from './RadioGraph.svelte';
	import { Waypoints, MapPin } from '@lucide/svelte';

	const params = browser ? new URLSearchParams(window.location.search) : undefined;
	const useMock = (params?.get('mockTopology') ?? params?.get('mockDirectory')) === '1';

	if (!useMock) {
		$effect(startTopologyPolling);
	}

	const graph = $derived<TopoGraph | undefined>(useMock ? mockTopologyGraph : topologyStore.graph);
	const loaded = $derived(useMock || topologyStore.loaded);
	const lastUpdated = $derived(useMock ? Date.now() : topologyStore.lastUpdated);

	const directory = $derived(directoryStore.directory);

	interface Row {
		key: string;
		name: string;
		fallback: string;
		subnet: string | null;
		isSelf: boolean;
	}

	function fallbackLabel(nodeId: string, subnet: string | null): string {
		const octet = subnet?.split('.')?.[2];
		if (octet) return `Router ${octet}`;
		return nodeId.length > 8 ? nodeId.slice(0, 8) : nodeId;
	}

	const rows = $derived.by<Row[]>(() => {
		// `directory.node` is null until the daemon writes its first projection
		// (fresh boot / unclaimed subnet). Guard it so the panel shows the empty
		// "No routers known yet." state instead of throwing (mjolnir-mesh-34z).
		if (!directory || !directory.node) return [];
		const self: Row = {
			key: directory.node.node_id,
			name: directory.node.name?.trim() ?? '',
			fallback: fallbackLabel(directory.node.node_id, directory.node.subnet),
			subnet: directory.node.subnet,
			isSelf: true
		};
		const neighbors = directory.neighbors.map((n) => ({
			key: n.node_id,
			name: n.name?.trim() ?? '',
			fallback: fallbackLabel(n.node_id, n.subnet),
			subnet: n.subnet,
			isSelf: false
		}));
		return [self, ...neighbors];
	});
</script>

<section
	class="flex flex-col gap-4 rounded-lg border border-border bg-card p-4 text-card-foreground"
>
	<header class="flex flex-col gap-1">
		<div class="flex items-center gap-2">
			<Waypoints class="size-4 text-muted-foreground" aria-hidden="true" />
			<h2 class="text-lg font-semibold">Routers</h2>
		</div>
		<p class="text-sm text-muted-foreground">
			The routers that make up this mesh. Signal lines show which ones talk to each other over the
			air.
		</p>
	</header>

	{#if !directoryStore.loaded}
		<p class="text-sm text-muted-foreground">Loading routers…</p>
	{:else if rows.length === 0}
		<p class="text-sm text-muted-foreground">No routers known yet.</p>
	{:else}
		<ul class="flex flex-col divide-y divide-border rounded-md border border-border">
			{#each rows as row (row.key)}
				<li class="flex items-center gap-3 px-3 py-2 text-sm">
					{#if row.name}
						<span class="font-medium">{row.name}</span>
					{:else}
						<span class="text-muted-foreground italic">{row.fallback}</span>
					{/if}
					{#if row.subnet}
						<span class="font-mono text-xs text-muted-foreground">{row.subnet}</span>
					{/if}
					{#if row.isSelf}
						<span class="ml-auto flex items-center gap-1 text-xs text-primary">
							<MapPin class="size-3.5" aria-hidden="true" />
							you are here
						</span>
					{/if}
				</li>
			{/each}
		</ul>

		<RadioGraph {graph} {loaded} {lastUpdated} />
	{/if}
</section>
