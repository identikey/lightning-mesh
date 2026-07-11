// Live topology poller (bead 4le). Rides on the existing directory poll for
// node/neighbor identity, then fetches /api/radio same-origin (self) and
// cross-origin from each neighbor every 10s to build a renderable graph.
// mjolnir-hello binds the node's LAN address, not the overlay backhaul addr,
// so neighbors are fetched at their routed LAN gateway (subnet's .1), with
// the backhaul addr kept as a fallback origin in case hello ever binds it.
// Runes-based, browser-only; call `startTopologyPolling()` from a component
// `$effect`.

import { browser } from '$app/environment';
import { directoryStore } from '$lib/directory/store.svelte';
import { fetchRadio, type RadioSnapshot } from './api';
import { buildTopology, type TopoGraph } from './graph';

const POLL_INTERVAL_MS = 10000;

function hostOf(addr: string | undefined): string | undefined {
	if (!addr) return undefined;
	const idx = addr.lastIndexOf(':');
	return idx === -1 ? addr : addr.slice(0, idx);
}

/** LAN gateway (`x.y.z.1`) of a node's client subnet, e.g. `10.42.242.0/24`. */
function lanGatewayOf(subnet: string | null | undefined): string | undefined {
	const octets = subnet?.split('/')[0]?.split('.');
	if (octets?.length !== 4) return undefined;
	return `${octets[0]}.${octets[1]}.${octets[2]}.1`;
}

class TopologyStore {
	graph = $state<TopoGraph | undefined>(undefined);
	loaded = $state(false);
	lastUpdated = $state<number | undefined>(undefined);

	async poll() {
		const directory = directoryStore.directory;
		// `node` is null until the daemon writes its first projection (fresh boot /
		// unclaimed subnet). buildTopology dereferences directory.node, so bail here
		// rather than throw — the panel keeps its last graph / "loading" state
		// instead of going permanently blank (mjolnir-mesh-34z).
		if (!directory || !directory.node) return;

		const port = browser ? window.location.port : '';
		const protocol = browser ? window.location.protocol : 'http:';
		const originFor = (host: string) => `${protocol}//${host}${port ? `:${port}` : ''}`;

		const targets: { key: string; origin: string | undefined }[] = [
			{ key: directory.node.backhaul_addr, origin: '' },
			...directory.neighbors.map((n) => {
				const host = hostOf(n.addrs[0]);
				const fetchHost = lanGatewayOf(n.subnet) ?? host;
				return { key: host ?? n.node_id, origin: fetchHost ? originFor(fetchHost) : undefined };
			})
		];

		const entries = await Promise.all(
			targets.map(async ({ key, origin }): Promise<[string, RadioSnapshot | undefined]> => {
				if (origin === undefined) return [key, undefined];
				return [key, await fetchRadio(origin)];
			})
		);

		this.graph = buildTopology(directory, new Map(entries));
		this.lastUpdated = Date.now();
		this.loaded = true;
	}
}

export const topologyStore = new TopologyStore();

/** Begin polling; returns a teardown to hand back from `$effect`. */
export function startTopologyPolling(): () => void {
	if (!browser) return () => {};
	topologyStore.poll();
	const interval = setInterval(() => topologyStore.poll(), POLL_INTERVAL_MS);
	return () => clearInterval(interval);
}
