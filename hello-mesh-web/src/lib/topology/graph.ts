// Pure graph builder: turns a directory snapshot plus a map of per-node radio
// telemetry into a renderable topology graph. No fetching, no DOM — kept
// testable and reusable by both the live store and the mock fixture.

import type { Directory } from '$lib/directory/api';
import type { RadioSnapshot } from './api';

export interface TopoNode {
	/** Stable key: the node's backhaul (overlay) address. */
	key: string;
	/** Short human label — subnet's third octet, falling back to a short node id. */
	label: string;
	nodeId: string;
	subnet: string | null;
	isSelf: boolean;
	/** Present when this node answered /api/radio; absent -> "no telemetry". */
	radio?: RadioSnapshot;
}

export interface TopoEdgeDirection {
	throughputMbps: number;
	/** True when the owning node's HWMP next-hop for this peer isn't the peer itself. */
	relayed: boolean;
}

export interface TopoEdge {
	/** `a`/`b` are node keys, sorted so each unordered pair has one edge. */
	a: string;
	b: string;
	/** Telemetry as seen from `a` looking at `b`, if `a` reported it. */
	aToB?: TopoEdgeDirection;
	/** Telemetry as seen from `b` looking at `a`, if `b` reported it. */
	bToA?: TopoEdgeDirection;
}

export interface TopoGraph {
	nodes: TopoNode[];
	edges: TopoEdge[];
}

function hostOf(addr: string | undefined): string | undefined {
	if (!addr) return undefined;
	const idx = addr.lastIndexOf(':');
	return idx === -1 ? addr : addr.slice(0, idx);
}

function shortId(id: string): string {
	return id.length > 8 ? id.slice(0, 8) : id;
}

function labelFor(nodeId: string, subnet: string | null): string {
	if (subnet) {
		const octets = subnet.split('.');
		if (octets.length >= 3 && octets[2]) return octets[2];
	}
	return shortId(nodeId);
}

/** Classification used for edge styling; exported so the panel and any tests agree. */
export type EdgeStrength = 'strong' | 'relay' | 'weak';

const STRONG_MBPS = 300;
const WEAK_MBPS = 100;

export function edgeStrength(edge: TopoEdge): EdgeStrength {
	const dirs = [edge.aToB, edge.bToA].filter((d): d is TopoEdgeDirection => !!d);
	if (dirs.length === 0) return 'weak';
	if (dirs.every((d) => d.relayed)) return 'weak';
	const maxMbps = Math.max(...dirs.map((d) => d.throughputMbps));
	if (maxMbps >= STRONG_MBPS) return 'strong';
	if (maxMbps < WEAK_MBPS) return 'weak';
	return 'relay';
}

export function buildTopology(
	directory: Directory,
	radioByKey: Map<string, RadioSnapshot | undefined>
): TopoGraph {
	const nodes: TopoNode[] = [];
	const selfKey = directory.node.backhaul_addr;

	nodes.push({
		key: selfKey,
		label: labelFor(directory.node.node_id, directory.node.subnet),
		nodeId: directory.node.node_id,
		subnet: directory.node.subnet,
		isSelf: true,
		radio: radioByKey.get(selfKey)
	});

	for (const neighbor of directory.neighbors) {
		const key = hostOf(neighbor.addrs[0]) ?? neighbor.node_id;
		if (nodes.some((n) => n.key === key)) continue;
		nodes.push({
			key,
			label: labelFor(neighbor.node_id, neighbor.subnet),
			nodeId: neighbor.node_id,
			subnet: neighbor.subnet,
			isSelf: false,
			radio: radioByKey.get(key)
		});
	}

	// mesh MAC -> node key, restricted to nodes we actually heard telemetry from.
	const macToKey = new Map<string, string>();
	for (const n of nodes) {
		if (n.radio?.mesh_mac) macToKey.set(n.radio.mesh_mac.toLowerCase(), n.key);
	}

	const edgeMap = new Map<string, TopoEdge>();
	function getEdge(x: string, y: string): TopoEdge {
		const [a, b] = [x, y].sort();
		const id = `${a}|${b}`;
		let e = edgeMap.get(id);
		if (!e) {
			e = { a, b };
			edgeMap.set(id, e);
		}
		return e;
	}

	for (const n of nodes) {
		if (!n.radio) continue;
		const nextHopByDst = new Map<string, string>();
		for (const mp of n.radio.mpaths ?? []) {
			nextHopByDst.set(mp.dst.toLowerCase(), mp.next_hop.toLowerCase());
		}
		for (const st of n.radio.stations ?? []) {
			const peerKey = macToKey.get(st.mac.toLowerCase());
			if (!peerKey || peerKey === n.key) continue;
			const nextHop = nextHopByDst.get(st.mac.toLowerCase());
			const relayed = !!nextHop && nextHop !== st.mac.toLowerCase();
			const edge = getEdge(n.key, peerKey);
			const dir: TopoEdgeDirection = { throughputMbps: st.expected_throughput_mbps, relayed };
			if (edge.a === n.key) edge.aToB = dir;
			else edge.bToA = dir;
		}
	}

	return { nodes, edges: [...edgeMap.values()] };
}
