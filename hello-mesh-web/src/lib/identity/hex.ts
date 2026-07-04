// Hex encode/decode helpers for Ed25519 key/signature bytes exchanged with
// the /api/challenge and /api/identity JSON endpoints (S4, bead 5zn).

export function bytesToHex(bytes: Uint8Array): string {
	let hex = '';
	for (const byte of bytes) {
		hex += byte.toString(16).padStart(2, '0');
	}
	return hex;
}

export function hexToBytes(hex: string): Uint8Array {
	if (hex.length % 2 !== 0) {
		throw new Error(`invalid hex string (odd length): ${hex.length}`);
	}
	const bytes = new Uint8Array(hex.length / 2);
	for (let i = 0; i < bytes.length; i++) {
		const byte = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
		if (Number.isNaN(byte)) {
			throw new Error(`invalid hex string: ${hex}`);
		}
		bytes[i] = byte;
	}
	return bytes;
}
