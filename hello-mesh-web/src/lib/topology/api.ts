// Client for the per-node radio telemetry endpoint (bead 4le): GET /api/radio.
// Same-origin for the local node; cross-origin (CORS-enabled server-side) for
// every neighbor at http://<backhaul_addr>:<page-port>/api/radio. May 404 on
// nodes that haven't been upgraded yet — that's a normal, expected outcome,
// not an error to surface.
//
// Schema v1 is a FIXED CONTRACT (do not rename fields):
export interface RadioStation {
	mac: string;
	signal_dbm: number;
	expected_throughput_mbps: number;
	inactive_ms: number;
}

export interface RadioMpath {
	dst: string;
	next_hop: string;
	metric: number;
}

export interface RadioSnapshot {
	version: number;
	backhaul_addr: string;
	mesh_if: string;
	mesh_mac: string;
	channel: number;
	freq_mhz: number;
	collected_at_unix: number;
	stations: RadioStation[];
	mpaths: RadioMpath[];
}

const FETCH_TIMEOUT_MS = 3000;

/**
 * Fetch `${origin}/api/radio`. `origin` may be `''` for same-origin (path-only)
 * requests. Fails open: 404, network errors, and timeouts all resolve to
 * `undefined` rather than throwing — a node with no telemetry is a normal
 * state the caller renders as "no telemetry", not a poll failure.
 */
export async function fetchRadio(
	origin: string,
	fetchImpl: typeof fetch = fetch
): Promise<RadioSnapshot | undefined> {
	const controller = new AbortController();
	const timeout = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS);
	try {
		const res = await fetchImpl(`${origin}/api/radio`, { signal: controller.signal });
		if (!res.ok) return undefined;
		return (await res.json()) as RadioSnapshot;
	} catch {
		return undefined;
	} finally {
		clearTimeout(timeout);
	}
}
