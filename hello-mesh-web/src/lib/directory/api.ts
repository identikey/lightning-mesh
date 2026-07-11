// Client for the mjolnir-hello read-only directory API (S3, bead 11l):
// GET /api/directory. Relative path — served by whichever node answers
// hello.mesh, no external hosts.

export interface DirectoryNode {
	node_id: string;
	subnet: string | null;
	backhaul_addr: string;
	/** Human router name, if one has been set (additive; older daemons omit it). */
	name?: string;
}

export interface DirectoryNeighbor {
	node_id: string;
	addrs: string[];
	subnet: string | null;
	/** Human router name, if one has been set (additive; older daemons omit it). */
	name?: string;
}

export interface DirectoryIdentity {
	username: string;
	display_name: string;
	/** ms since epoch of this identity's last announce (writer clock, approximate;
	    additive — older daemons omit it, in which case recency is unknown). */
	last_seen_unix?: number;
}

export interface DirectoryService {
	/** Fully-qualified advertised service name, e.g. `printer._ipp._tcp`. */
	name: string;
	ip: string;
	port: number;
	protocol: string;
	/** Advertised instance hostname (absent on owner-bound v2 records). */
	hostname?: string;
	/** Advertised `key=value` TXT records (absent/empty omitted by the daemon). */
	txt?: Record<string, string>;
	/** Advertising device MAC as colon-hex, e.g. `de:ad:be:ef:00:01`. */
	host_mac?: string;
}

export interface Directory {
	version: number;
	/** `null` when the daemon hasn't written a projection yet (fresh boot,
	    unclaimed subnet) — the server serves `{"node":null,...}` in that window.
	    Consumers must treat null as "initializing", never dereference it. */
	node: DirectoryNode | null;
	neighbors: DirectoryNeighbor[];
	identities: DirectoryIdentity[];
	services: DirectoryService[];
}

export async function fetchDirectory(fetchImpl: typeof fetch = fetch): Promise<Directory> {
	const res = await fetchImpl('/api/directory');
	if (!res.ok) {
		throw new Error(`GET /api/directory failed: ${res.status} ${res.statusText}`);
	}
	return (await res.json()) as Directory;
}
