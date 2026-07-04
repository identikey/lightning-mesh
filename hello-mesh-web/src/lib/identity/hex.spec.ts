import { describe, expect, it } from 'vitest';
import { bytesToHex, hexToBytes } from './hex';

describe('hex encode/decode', () => {
	it('round-trips arbitrary bytes', () => {
		const bytes = new Uint8Array([0, 1, 2, 253, 254, 255, 16, 128]);
		expect(hexToBytes(bytesToHex(bytes))).toEqual(bytes);
	});

	it('lower-cases and zero-pads single-digit bytes', () => {
		expect(bytesToHex(new Uint8Array([0, 10, 255]))).toBe('000aff');
	});

	it('rejects odd-length hex strings', () => {
		expect(() => hexToBytes('abc')).toThrow();
	});

	it('rejects non-hex characters', () => {
		expect(() => hexToBytes('zz')).toThrow();
	});
});
