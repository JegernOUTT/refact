/* eslint-disable @typescript-eslint/naming-convention */
import { createHash } from 'crypto';
import { verifyVsix } from './hostReleaseArchive';
export type ReleaseKey = { prepared_hash: string; grant_id: string; project_id: string; host_session_id: string };
export type InstalledIdentity = { manifest_digest: string; version: string };
export type RunningIdentity = InstalledIdentity & { host_session_id: string };
export type HostReleaseRequest = ReleaseKey & {
    schema_version: number; target: string; operation: 'install' | 'recover'; candidate_id: string;
    candidate_digest: string; proof_id: string; artifact_digest: string; manifest_digest: string;
    package_version: string; expected_installed_manifest_digest: string; expected_running_manifest_digest: string;
    files: Record<string, string>;
};
export type HostReleaseReceipt = {
    schema_version: number; prepared_hash: string; grant_id: string; host_session_id: string; artifact_digest: string;
    status: 'declined' | 'installer_returned' | 'outcome_unknown'; installed: InstalledIdentity | null; running: RunningIdentity | null;
};
export type ReleaseClaim = { key: ReleaseKey; receipt: HostReleaseReceipt };
export interface ReleaseTransport {
    fetch(key: ReleaseKey): Promise<{ request: HostReleaseRequest; bytes: Buffer }>;
    report(key: ReleaseKey, receipt: HostReleaseReceipt): Promise<void>;
    reconcile(key: ReleaseKey, installed: InstalledIdentity | null, running: RunningIdentity): Promise<void>;
}
export interface ReleaseHost {
    readonly running: RunningIdentity;
    installed(): Promise<InstalledIdentity>;
    confirm(request: HostReleaseRequest): Promise<boolean>;
    stage(bytes: Buffer): Promise<{ install(): Promise<void>; dispose(): Promise<void> }>;
    claims(): Promise<ReleaseClaim[]>;
    claim(claim: ReleaseClaim): Promise<boolean>;
    save(claim: ReleaseClaim): Promise<void>;
}
export const sha256 = (bytes: Buffer | string): string => createHash('sha256').update(bytes).digest('hex');
const digest = (s: unknown): s is string => typeof s === 'string' && /^[a-f0-9]{64}$/.test(s);
export function validKey(value: unknown): value is ReleaseKey {
    if (!value || typeof value !== 'object') { return false; }
    const k = value as ReleaseKey;
    return Object.keys(k).sort().join(',') === 'grant_id,host_session_id,prepared_hash,project_id' && digest(k.prepared_hash)
        && [k.grant_id, k.project_id, k.host_session_id].every(v => typeof v === 'string' && /^[A-Za-z0-9_.:-]{1,128}$/.test(v));
}
export function safeReleasePath(p: string): boolean {
    return p.length > 0 && Buffer.byteLength(p) < 4096 && !/[\\\u0000-\u001f\u007f:]/.test(p)
        && p.split('/').every(v => v !== '' && v !== '.' && v !== '..');
}
export function manifestDigest(files: Record<string, string>): string {
    const keys = Object.keys(files).sort((a, b) => Buffer.compare(Buffer.from(a), Buffer.from(b)));
    if (!keys.length || keys.length > 20000 || keys.some(k => !safeReleasePath(k) || k === 'package.json' || !digest(files[k]))) { throw new Error('Invalid file manifest'); }
    // Exact serde_json BTreeMap encoding: JS object integer-key ordering is not equivalent.
    return sha256('{' + keys.map(k => JSON.stringify(k) + ':' + JSON.stringify(files[k])).join(',') + '}');
}
export { crc32, verifyVsix } from './hostReleaseArchive';
export class HostReleaseOwner {
    private busy = false;
    constructor(private readonly host: ReleaseHost, private readonly transport: ReleaseTransport) {}
    async run(key: ReleaseKey): Promise<HostReleaseReceipt> {
        if (!validKey(key) || key.host_session_id !== this.host.running.host_session_id || this.busy) { throw new Error('Invalid host/session or release in progress'); }
        this.busy = true;
        try {
            if ((await this.host.claims()).some(c => c.key.prepared_hash === key.prepared_hash)) { throw new Error('Release already claimed; reconcile only'); }
            const { request: r, bytes } = await this.transport.fetch(key);
            const fields = [...Object.keys(key), 'schema_version', 'target', 'operation', 'candidate_id', 'candidate_digest', 'proof_id', 'artifact_digest', 'manifest_digest', 'package_version', 'expected_installed_manifest_digest', 'expected_running_manifest_digest', 'files'].sort();
            if (!r || Object.keys(r).sort().join(',') !== fields.join(',') || r.schema_version !== 1 || r.target !== 'vscode' || !['install', 'recover'].includes(r.operation)
                || Object.keys(key).some(k => r[k as keyof ReleaseKey] !== key[k as keyof ReleaseKey])
                || !digest(r.artifact_digest) || !digest(r.candidate_digest) || !digest(r.manifest_digest)
                || !digest(r.expected_installed_manifest_digest) || !digest(r.expected_running_manifest_digest)
                || typeof r.package_version !== 'string' || !/^\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?(?:\+[A-Za-z0-9.-]+)?$/.test(r.package_version)
                || !r.files || manifestDigest(r.files) !== r.manifest_digest) { throw new Error('Release binding mismatch'); }
            verifyVsix(bytes, r);
            const installed = await this.host.installed();
            if (installed.manifest_digest !== r.expected_installed_manifest_digest || this.host.running.manifest_digest !== r.expected_running_manifest_digest) { throw new Error('Current installed/running manifest mismatch'); }
            const receipt: HostReleaseReceipt = { schema_version: 1, prepared_hash: key.prepared_hash, grant_id: key.grant_id,
                host_session_id: key.host_session_id, artifact_digest: r.artifact_digest, status: 'declined', installed, running: { ...this.host.running } };
            if (!(await this.host.confirm(r))) {
                if (!(await this.host.claim({ key, receipt }))) { throw new Error('Release already claimed'); }
                await this.transport.report(key, receipt); return receipt;
            }
            const staged = await this.host.stage(bytes);
            try {
                if ((await this.host.installed()).manifest_digest !== installed.manifest_digest) { throw new Error('Installed generation changed during confirmation'); }
                receipt.status = 'outcome_unknown';
                if (!(await this.host.claim({ key, receipt }))) { throw new Error('Release already claimed'); }
                // Durable unknown claim precedes the supported host effect; never retry that effect.
                try { await staged.install(); receipt.status = 'installer_returned'; } catch { /* Rejection cannot prove no effect. */ }
                try { receipt.installed = await this.host.installed(); } catch { receipt.installed = null; }
                await this.host.save({ key, receipt }); await this.transport.report(key, receipt); return receipt;
            } finally { await staged.dispose(); }
        } finally { this.busy = false; }
    }
    async reconcile(): Promise<void> {
        for (const claim of await this.host.claims()) {
            await this.transport.report(claim.key, claim.receipt);
            let installed: InstalledIdentity | null = null;
            try { installed = await this.host.installed(); } catch { /* fail closed */ }
            await this.transport.reconcile(claim.key, installed, { ...this.host.running });
        }
    }
}
