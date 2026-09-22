/* eslint-disable @typescript-eslint/naming-convention */
import { inflateRawSync } from 'zlib';
import { HostReleaseRequest, manifestDigest, safeReleasePath, sha256 } from './hostRelease';

export function crc32(bytes: Buffer): number {
    let crc = 0xffffffff;
    for (const byte of bytes) {
        crc ^= byte;
        for (let bit = 0; bit < 8; bit++) { crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0); }
    }
    return (crc ^ 0xffffffff) >>> 0;
}
// Inspect in memory only. Never extract paths or invoke an archive command.
export function verifyVsix(bytes: Buffer, request: HostReleaseRequest): void {
    if (bytes.length > 256 * 1024 * 1024 || sha256(bytes) !== request.artifact_digest) { throw new Error('Artifact SHA mismatch'); }
    let end = -1;
    for (let i = bytes.length - 22; i >= Math.max(0, bytes.length - 65557); i--) {
        if (bytes.readUInt32LE(i) === 0x06054b50 && i + 22 + bytes.readUInt16LE(i + 20) === bytes.length) { end = i; break; }
    }
    if (end < 0 || bytes.readUInt16LE(end + 4) !== 0 || bytes.readUInt16LE(end + 6) !== 0) { throw new Error('Invalid ZIP'); }
    const count = bytes.readUInt16LE(end + 10), centralStart = bytes.readUInt32LE(end + 16);
    let cursor = centralStart;
    const centralEnd = cursor + bytes.readUInt32LE(end + 12);
    if (!count || count > 20000 || centralEnd !== end || bytes.readUInt16LE(end + 8) !== count) { throw new Error('Invalid ZIP directory'); }
    const files: Record<string, string> = Object.create(null), names = new Set<string>();
    const ranges: [number, number][] = [];
    let packageVersion: string | undefined, total = 0;
    for (let n = 0; n < count; n++) {
        if (cursor + 46 > end || bytes.readUInt32LE(cursor) !== 0x02014b50) { throw new Error('Invalid ZIP entry'); }
        const flags = bytes.readUInt16LE(cursor + 8), method = bytes.readUInt16LE(cursor + 10);
        const compressed = bytes.readUInt32LE(cursor + 20), size = bytes.readUInt32LE(cursor + 24);
        const nameSize = bytes.readUInt16LE(cursor + 28), extra = bytes.readUInt16LE(cursor + 30), comment = bytes.readUInt16LE(cursor + 32);
        const offset = bytes.readUInt32LE(cursor + 42);
        const nameBytes = bytes.subarray(cursor + 46, cursor + 46 + nameSize), name = nameBytes.toString('utf8');
        const directory = name.endsWith('/');
        if (cursor + 46 + nameSize + extra + comment > end || !Buffer.from(name).equals(nameBytes)
            || !safeReleasePath(directory ? name.slice(0, -1) : name) || names.has(name.toLowerCase()) || (flags & ~(0x800 | 8 | 2 | 4)) !== 0
            || ![0, 8].includes(method) || bytes.readUInt16LE(cursor + 34) !== 0
            || ((bytes.readUInt32LE(cursor + 38) >>> 16) & 0xf000) === 0xa000) { throw new Error('Unsafe ZIP entry'); }
        names.add(name.toLowerCase());
        if (offset + 30 > centralStart || bytes.readUInt32LE(offset) !== 0x04034b50) { throw new Error('Invalid ZIP local header'); }
        const localNameSize = bytes.readUInt16LE(offset + 26), localExtra = bytes.readUInt16LE(offset + 28);
        const start = offset + 30 + localNameSize + localExtra, finish = start + compressed;
        if (!bytes.subarray(offset + 30, offset + 30 + localNameSize).equals(nameBytes)
            || bytes.readUInt16LE(offset + 8) !== method || bytes.readUInt16LE(offset + 6) !== flags
            || finish > centralStart || ranges.some(([a, b]) => offset < b && finish > a)) { throw new Error('ZIP local header mismatch'); }
        ranges.push([offset, finish]);
        total += size;
        if (total > 512 * 1024 * 1024 || size > 128 * 1024 * 1024) { throw new Error('ZIP size limit'); }
        const raw = bytes.subarray(start, finish);
        const data = method === 0 ? raw : inflateRawSync(raw, { maxOutputLength: Math.max(1, size) });
        if (data.length !== size || crc32(data) !== bytes.readUInt32LE(cursor + 16)) { throw new Error('ZIP size/CRC mismatch'); }
        if (!(flags & 8) && (bytes.readUInt32LE(offset + 14) !== bytes.readUInt32LE(cursor + 16)
            || bytes.readUInt32LE(offset + 18) !== compressed || bytes.readUInt32LE(offset + 22) !== size)) { throw new Error('ZIP local sizes mismatch'); }
        if (!directory && name.startsWith('extension/')) {
            const relative = name.slice('extension/'.length);
            if (relative === 'package.json') {
                const pkg = JSON.parse(data.toString('utf8')) as { publisher: string; name: string; version: string };
                if (pkg.publisher !== 'smallcloud' || pkg.name !== 'codify') { throw new Error('Package identity mismatch'); }
                packageVersion = pkg.version;
            } else { files[relative] = sha256(data); }
        }
        cursor += 46 + nameSize + extra + comment;
    }
    if (cursor !== centralEnd || packageVersion !== request.package_version || manifestDigest(files) !== request.manifest_digest) { throw new Error('Package manifest/version mismatch'); }
}
