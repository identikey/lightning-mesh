// Hardcoded 4-node mock graph for previewing TopologyPanel without a live
// mesh. Mirrors the reference walk: an egress node, "you are here", a fresh
// relay, and a far node that only reaches the others through that relay.
// Enabled in the app via the `?mockTopology=1` query param (dev/preview only).

import type { TopoGraph } from './graph';

export const mockTopologyGraph: TopoGraph = {
	nodes: [
		{
			key: '10.254.242.84',
			label: '243',
			nodeId: 'wr3000s-a',
			subnet: '10.42.243.0/24',
			isSelf: true,
			radio: {
				version: 1,
				backhaul_addr: '10.254.242.84',
				mesh_if: 'phy1-mesh0',
				mesh_mac: 'aa:bb:cc:d9:85:af',
				channel: 36,
				freq_mhz: 5180,
				collected_at_unix: 1751234567,
				stations: [],
				mpaths: []
			}
		},
		{
			key: '10.254.12.214',
			label: '12',
			nodeId: 'm3000-b',
			subnet: '10.42.12.0/24',
			isSelf: false,
			radio: {
				version: 1,
				backhaul_addr: '10.254.12.214',
				mesh_if: 'phy1-mesh0',
				mesh_mac: 'aa:bb:cc:e7:ba:9d',
				channel: 36,
				freq_mhz: 5180,
				collected_at_unix: 1751234567,
				stations: [],
				mpaths: []
			}
		},
		{
			key: '10.254.61.115',
			label: '61',
			nodeId: 'tr3000',
			subnet: '10.42.61.0/24',
			isSelf: false,
			radio: {
				version: 1,
				backhaul_addr: '10.254.61.115',
				mesh_if: 'phy1-mesh0',
				mesh_mac: 'aa:bb:cc:98:fb:10',
				channel: 36,
				freq_mhz: 5180,
				collected_at_unix: 1751234567,
				stations: [],
				mpaths: []
			}
		},
		{
			key: '10.254.242.172',
			label: '242',
			nodeId: 'm3000',
			subnet: '10.42.242.0/24',
			isSelf: false,
			radio: undefined // "no telemetry" node — reachable via directory, /api/radio 404s
		}
	],
	edges: [
		{
			a: '10.254.12.214',
			b: '10.254.242.84',
			aToB: { throughputMbps: 979, relayed: false },
			bToA: { throughputMbps: 979, relayed: false }
		},
		{
			a: '10.254.12.214',
			b: '10.254.61.115',
			aToB: { throughputMbps: 552, relayed: false },
			bToA: { throughputMbps: 609, relayed: false }
		},
		{
			a: '10.254.242.84',
			b: '10.254.61.115',
			aToB: { throughputMbps: 493, relayed: false },
			bToA: { throughputMbps: 724, relayed: false }
		},
		{
			a: '10.254.61.115',
			b: '10.254.242.172',
			aToB: { throughputMbps: 246, relayed: false },
			bToA: { throughputMbps: 5, relayed: false }
		},
		{
			a: '10.254.12.214',
			b: '10.254.242.172',
			aToB: { throughputMbps: 94, relayed: true },
			bToA: { throughputMbps: 31, relayed: true }
		},
		{
			a: '10.254.242.84',
			b: '10.254.242.172',
			aToB: { throughputMbps: 15, relayed: true },
			bToA: { throughputMbps: 5, relayed: true }
		}
	]
};
