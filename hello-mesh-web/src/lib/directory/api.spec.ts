import { describe, expect, it, vi } from 'vitest';
import { fetchDirectory, type Directory } from './api';

const sampleDirectory: Directory = {
	version: 1,
	node: { node_id: 'aaaa1111', subnet: '10.42.1.0/24', backhaul_addr: '10.254.1.1' },
	neighbors: [{ node_id: 'bbbb2222', addrs: ['10.254.2.1'], subnet: '10.42.2.0/24' }],
	identities: [{ username: 'alice', display_name: 'Alice' }],
	services: [{ name: 'moq-relay', ip: '10.42.1.5', port: 4433, protocol: 'quic' }]
};

function fakeFetch(response: Partial<Response> & { ok: boolean; json?: () => Promise<unknown> }) {
	return vi.fn().mockResolvedValue(response as Response);
}

describe('fetchDirectory', () => {
	it('parses a well-formed directory response', async () => {
		const fetchImpl = fakeFetch({ ok: true, json: async () => sampleDirectory });
		const directory = await fetchDirectory(fetchImpl);
		expect(directory).toEqual(sampleDirectory);
		expect(fetchImpl).toHaveBeenCalledWith('/api/directory');
	});

	it('throws when the response is not ok', async () => {
		const fetchImpl = fakeFetch({ ok: false, status: 500, statusText: 'Internal Server Error' });
		await expect(fetchDirectory(fetchImpl)).rejects.toThrow('GET /api/directory failed');
	});
});
