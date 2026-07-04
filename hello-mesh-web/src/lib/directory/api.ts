// Client for the mjolnir-hello read-only directory API (S3, bead 11l):
// GET /api/directory. Relative path — served by whichever node answers
// hello.mesh, no external hosts.

export interface DirectoryNode {
	node_id: string;
	subnet: string | null;
	backhaul_addr: string;
}

export interface DirectoryNeighbor {
	node_id: string;
	addrs: string[];
	subnet: string | null;
}

export interface DirectoryIdentity {
	username: string;
	display_name: string;
}

export interface DirectoryService {
	name: string;
	ip: string;
	port: number;
	protocol: string;
}

export interface Directory {
	version: number;
	node: DirectoryNode;
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
