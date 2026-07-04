// Client for the mjolnir-hello identity API (S4, bead 5zn): GET /api/challenge,
// POST /api/identity. Relative paths — served by whichever node answers
// hello.mesh, no external hosts.

export async function fetchChallenge(fetchImpl: typeof fetch = fetch): Promise<string> {
	const res = await fetchImpl('/api/challenge');
	if (!res.ok) {
		throw new Error(`GET /api/challenge failed: ${res.status} ${res.statusText}`);
	}
	const body = (await res.json()) as { challenge: string };
	return body.challenge;
}

export interface SubmitIdentityParams {
	pubkey: string;
	sig: string;
	challenge: string;
	label?: string;
}

export async function submitIdentity(
	params: SubmitIdentityParams,
	fetchImpl: typeof fetch = fetch
): Promise<void> {
	const res = await fetchImpl('/api/identity', {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify(params)
	});
	if (!res.ok) {
		const detail = await res.text().catch(() => '');
		throw new Error(`POST /api/identity failed: ${res.status} ${res.statusText} ${detail}`);
	}
}
