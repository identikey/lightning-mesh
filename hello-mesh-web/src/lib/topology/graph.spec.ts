import { describe, it, expect } from 'vitest';
import { buildTopology } from './graph';
import type { Directory } from '$lib/directory/api';

const emptyRadio = new Map();

describe('buildTopology', () => {
	it('degrades to an empty graph when node is null (mjolnir-mesh-34z)', () => {
		// The daemon serves {"node":null,...} until it writes its first projection
		// (fresh boot / unclaimed subnet). buildTopology must not throw on it.
		const directory: Directory = {
			version: 1,
			node: null,
			neighbors: [],
			identities: [],
			services: []
		};
		const graph = buildTopology(directory, emptyRadio);
		expect(graph).toEqual({ nodes: [], edges: [] });
	});

	it('always renders the self node even with no neighbors', () => {
		const directory: Directory = {
			version: 1,
			node: {
				node_id: 'self-id',
				subnet: '10.42.12.0/24',
				backhaul_addr: '10.254.12.214',
				name: 'm3000-b'
			},
			neighbors: [],
			identities: [],
			services: []
		};
		const graph = buildTopology(directory, emptyRadio);
		expect(graph.nodes).toHaveLength(1);
		expect(graph.nodes[0]).toMatchObject({ isSelf: true, name: 'm3000-b' });
	});

	it('adds a node per neighbor', () => {
		const directory: Directory = {
			version: 1,
			node: {
				node_id: 'self-id',
				subnet: '10.42.12.0/24',
				backhaul_addr: '10.254.12.214',
				name: 'm3000-b'
			},
			neighbors: [
				{ node_id: 'n2', addrs: ['10.254.242.172:49737'], subnet: '10.42.242.0/24', name: 'm3000' }
			],
			identities: [],
			services: []
		};
		const graph = buildTopology(directory, emptyRadio);
		expect(graph.nodes).toHaveLength(2);
		expect(graph.nodes.filter((n) => !n.isSelf)).toHaveLength(1);
	});
});
