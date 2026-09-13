/* eslint-disable @typescript-eslint/naming-convention */
import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import * as http from 'http';
import { randomUUID } from 'crypto';
import { HostReleaseOwner, HostReleaseRequest, HostReleaseReceipt, InstalledIdentity, ReleaseClaim, ReleaseKey, ReleaseTransport, RunningIdentity, manifestDigest, sha256, validKey } from './hostRelease';

export function installedIdentity(root: string, version: string): InstalledIdentity {
    const files: Record<string, string> = Object.create(null);
    let total = 0, count = 0;
    function visit(relative: string): void {
        const dir = path.join(root, relative);
        if (!fs.lstatSync(dir).isDirectory() || fs.lstatSync(dir).isSymbolicLink()) { throw new Error('Unsafe installed directory'); }
        for (const name of fs.readdirSync(dir)) {
            const rel = relative ? relative + '/' + name : name;
            const file = path.join(root, rel), stat = fs.lstatSync(file);
            if (stat.isSymbolicLink()) { throw new Error('Installed symlink'); }
            if (stat.isDirectory()) { visit(rel); }
            else if (stat.isFile()) {
                if (rel === 'package.json') { continue; }
                total += stat.size;
                if (++count > 20000 || total > 512 * 1024 * 1024) { throw new Error('Installed manifest limit'); }
                const fd = fs.openSync(file, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
                try { files[rel] = sha256(fs.readFileSync(fd)); } finally { fs.closeSync(fd); }
            } else { throw new Error('Special installed file'); }
        }
    }
    visit('');
    return { manifest_digest: manifestDigest(files), version };
}

// Claims are never evicted to make room: a full ledger fails closed. A crash
// during creation may leave a malformed file/lock, which also fails closed.
export class FileReleaseClaims {
    constructor(private readonly root: string) {
        fs.mkdirSync(root, { recursive: true, mode: 0o700 });
        const stat = fs.lstatSync(root);
        if (!stat.isDirectory() || stat.isSymbolicLink()) { throw new Error('Unsafe claim directory'); }
        fs.chmodSync(root, 0o700);
    }
    private file(key: ReleaseKey): string {
        if (!validKey(key)) { throw new Error('Invalid claim key'); }
        return path.join(this.root, key.prepared_hash + '.json');
    }
    async list(): Promise<ReleaseClaim[]> {
        const names = fs.readdirSync(this.root).filter(n => n.endsWith('.json'));
        if (names.length > 128) { throw new Error('Claim ledger full'); }
        return names.map(n => {
            if (!/^[a-f0-9]{64}\.json$/.test(n)) { throw new Error('Invalid claim filename'); }
            const file = path.join(this.root, n), stat = fs.lstatSync(file);
            if (!stat.isFile() || stat.isSymbolicLink() || stat.size > 16384) { throw new Error('Invalid claim'); }
            const claim = JSON.parse(fs.readFileSync(file, 'utf8')) as ReleaseClaim;
            if (!validKey(claim.key) || n !== claim.key.prepared_hash + '.json' || !claim.receipt || claim.receipt.prepared_hash !== claim.key.prepared_hash) { throw new Error('Corrupt claim'); }
            return claim;
        });
    }
    private syncDirectory(): void {
        if (process.platform === 'win32') { return; }
        const fd = fs.openSync(this.root, 'r');
        try { fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
    }
    async create(claim: ReleaseClaim): Promise<boolean> {
        const lock = path.join(this.root, 'claim.lock');
        let fd: number;
        try { fd = fs.openSync(lock, 'wx', 0o600); } catch { throw new Error('Claim ledger locked; no installation allowed'); }
        try {
            if (fs.existsSync(this.file(claim.key))) { return false; }
            if ((await this.list()).length >= 128) { throw new Error('Claim ledger full; no automatic eviction'); }
            const out = fs.openSync(this.file(claim.key), 'wx', 0o600);
            try { fs.writeFileSync(out, JSON.stringify(claim)); fs.fsyncSync(out); } finally { fs.closeSync(out); }
            this.syncDirectory();
            return true;
        } finally { fs.closeSync(fd); fs.unlinkSync(lock); }
    }
    async save(claim: ReleaseClaim): Promise<void> {
        const target = this.file(claim.key), tmp = path.join(this.root, randomUUID() + '.tmp');
        const fd = fs.openSync(tmp, 'wx', 0o600);
        try { fs.writeFileSync(fd, JSON.stringify(claim)); fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
        fs.renameSync(tmp, target);
        this.syncDirectory();
    }
}

export class NativeReleaseTransport implements ReleaseTransport {
    constructor(private readonly port: number, private readonly basic: string, private readonly project: string) {
        if (!Number.isInteger(port) || port < 1 || port > 65535 || !basic.startsWith('Basic ')) { throw new Error('Native daemon authentication unavailable'); }
    }
    private post(key: ReleaseKey, action: 'artifact' | 'receipt' | 'observe', body: unknown): Promise<unknown> {
        if (!validKey(key) || key.project_id !== this.project) { return Promise.reject(new Error('Foreign project')); }
        const data = JSON.stringify(body);
        return new Promise((resolve, reject) => {
            const req = http.request({ hostname: '127.0.0.1', port: this.port, method: 'POST',
                path: `/daemon/v1/projects/${encodeURIComponent(this.project)}/host-release/${action}`,
                headers: { Authorization: this.basic, 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(data) } }, res => {
                const chunks: Buffer[] = []; let length = 0;
                res.on('data', (chunk: Buffer) => { length += chunk.length; if (length > 360 * 1024 * 1024) { req.destroy(new Error('Release response too large')); } else { chunks.push(chunk); } });
                res.on('error', reject);
                res.on('end', () => {
                    if (res.statusCode !== 200) { reject(new Error(`Native release endpoint rejected (${res.statusCode})`)); return; }
                    try { resolve(JSON.parse(Buffer.concat(chunks).toString('utf8'))); } catch (e) { reject(e); }
                });
            });
            req.setTimeout(120000, () => req.destroy(new Error('Native release transport timeout')));
            req.on('error', reject); req.end(data);
        });
    }
    async fetch(key: ReleaseKey): Promise<{ request: HostReleaseRequest; bytes: Buffer }> {
        const result = await this.post(key, 'artifact', { prepared_hash: key.prepared_hash, grant_id: key.grant_id }) as { request: HostReleaseRequest; artifact_base64: string };
        if (typeof result.artifact_base64 !== 'string') { throw new Error('Missing package bytes'); }
        const bytes = Buffer.from(result.artifact_base64, 'base64');
        if (bytes.toString('base64') !== result.artifact_base64) { throw new Error('Noncanonical package bytes'); }
        return { request: result.request, bytes };
    }
    async report(key: ReleaseKey, receipt: HostReleaseReceipt): Promise<void> { await this.post(key, 'receipt', receipt); }
    async reconcile(key: ReleaseKey, installed: InstalledIdentity | null, running: RunningIdentity): Promise<void> {
        if (!installed || running.host_session_id === key.host_session_id) { return; }
        // Artifact identity comes from the retained local native claim, not a GUI observation.
        const receipt = this.receipts?.find(c => c.key.prepared_hash === key.prepared_hash)?.receipt;
        if (!receipt) { throw new Error('Missing native reconciliation claim'); }
        await this.post(key, 'observe', { schema_version: 1, prepared_hash: key.prepared_hash, grant_id: key.grant_id, host_session_id: key.host_session_id, artifact_digest: receipt.artifact_digest, installed, running });
    }
    receipts?: ReleaseClaim[];
}

export function registerNativeHostRelease(context: vscode.ExtensionContext): void {
    // Synchronous activation-time snapshot BEFORE any command registration/install.
    let running: RunningIdentity;
    try { running = Object.freeze({ ...installedIdentity(context.extensionPath, String(context.extension.packageJSON.version)), host_session_id: randomUUID() }); }
    catch { console.warn('Native release unavailable: activation identity could not be captured'); return; }
    const ledger = new FileReleaseClaims(path.join(context.globalStorageUri.fsPath, 'host-release-claims'));
    const observe = async (): Promise<InstalledIdentity> => {
        const extension = vscode.extensions.getExtension(context.extension.id);
        if (!extension) { throw new Error('Installed extension unavailable'); }
        return installedIdentity(extension.extensionPath, String(extension.packageJSON.version));
    };
    const host = {
        running, installed: observe, claims: () => ledger.list(), claim: (c: ReleaseClaim) => ledger.create(c), save: (c: ReleaseClaim) => ledger.save(c),
        confirm: async (r: HostReleaseRequest): Promise<boolean> => (await vscode.window.showWarningMessage(
            `${r.operation === 'recover' ? 'Recover' : 'Install'} Refact ${r.package_version}?\nProject: ${r.project_id}\nArtifact SHA-256: ${r.artifact_digest}\nManifest: ${r.manifest_digest}\nThis changes the installed extension. Running code remains unchanged until you separately reload. No reload or disable will be performed.`,
            { modal: true }, 'Install verified package')) === 'Install verified package',
        stage: async (bytes: Buffer) => {
            const dir = fs.mkdtempSync(path.join(context.globalStorageUri.fsPath, 'release-stage-'));
            fs.chmodSync(dir, 0o700);
            const file = path.join(dir, 'verified.vsix');
            fs.writeFileSync(file, bytes, { mode: 0o600, flag: 'wx' });
            return {
                install: async () => {
                    if (sha256(fs.readFileSync(file)) !== sha256(bytes) || fs.lstatSync(file).isSymbolicLink()) { throw new Error('Staged bytes changed'); }
                    await vscode.commands.executeCommand('workbench.extensions.installExtension', vscode.Uri.file(file));
                },
                dispose: async () => { fs.rmSync(dir, { recursive: true, force: true }); }
            };
        }
    };
    let active = false;
    const connection = () => {
        const daemon = global.rust_binary_blob;
        if (!daemon?.port || !daemon.project_id) { throw new Error('Native project daemon not ready'); }
        return { port: daemon.port, project: daemon.project_id };
    };
    context.subscriptions.push(vscode.commands.registerCommand('refactaicmd.hostReleaseIdentity', async () => {
        const identity = { ...running };
        const selection = await vscode.window.showInformationMessage(
            `Refact ${identity.version}\nHost session: ${identity.host_session_id}\nRunning manifest: ${identity.manifest_digest}`,
            { modal: true }, 'Copy identity');
        if (selection === 'Copy identity') { await vscode.env.clipboard.writeText(JSON.stringify(identity)); }
        return identity;
    }));
    context.subscriptions.push(vscode.commands.registerCommand('refactaicmd.hostRelease', async (value: unknown) => {
        if (active) { throw new Error('Native release already active'); }
        active = true;
        try {
            const { port, project } = connection();
            if (value === undefined) {
                const prepared = await vscode.window.showInputBox({
                    title: 'Native release: prepared SHA-256', ignoreFocusOut: true,
                    validateInput: text => /^[a-f0-9]{64}$/.test(text.trim()) ? undefined : 'Enter the prepared release SHA-256.'
                });
                if (prepared === undefined) { return; }
                const grant = await vscode.window.showInputBox({
                    title: 'Native release: existing grant ID', ignoreFocusOut: true,
                    validateInput: text => /^[A-Za-z0-9_.:-]{1,128}$/.test(text.trim()) ? undefined : 'Enter the existing opaque grant ID.'
                });
                if (grant === undefined) { return; }
                value = { prepared_hash: prepared.trim(), grant_id: grant.trim(), project_id: project, host_session_id: running.host_session_id };
            }
            if (!validKey(value) || value.host_session_id !== running.host_session_id) { throw new Error('Expected opaque native release identifiers'); }
            if (value.project_id !== project) { throw new Error('Foreign project'); }
            const username = await vscode.window.showInputBox({ title: 'Native release: existing grant owner username', ignoreFocusOut: true });
            if (!username || username.includes(':')) { return; }
            const password = await vscode.window.showInputBox({ title: 'Native release: grant owner password (stored in VS Code SecretStorage for reconciliation)', password: true, ignoreFocusOut: true });
            if (!password) { return; }
            const basic = 'Basic ' + Buffer.from(username + ':' + password).toString('base64');
            await context.secrets.store(`host-release-basic:${port}:${project}`, basic);
            const transport = new NativeReleaseTransport(port, basic, project);
            return await new HostReleaseOwner(host, transport).run(value);
        } finally { active = false; }
    }));
    const reconcile = async () => {
        const { port, project } = connection();
        const basic = await context.secrets.get(`host-release-basic:${port}:${project}`);
        if (!basic) { return; }
        const transport = new NativeReleaseTransport(port, basic, project);
        transport.receipts = (await ledger.list()).filter(c => c.key.project_id === project);
        await new HostReleaseOwner({ ...host, claims: async () => transport.receipts ?? [] }, transport).reconcile();
    };
    context.subscriptions.push(vscode.commands.registerCommand('refactaicmd.hostReleaseReconcile', reconcile));
    // Bounded startup retry only reports observations; it NEVER dispatches installation.
    let attempts = 0;
    const timer = setInterval(() => {
        if (++attempts > 30) { clearInterval(timer); return; }
        if (global.rust_binary_blob?.project_id) { clearInterval(timer); void reconcile().catch(() => console.warn('Native release reconciliation pending')); }
    }, 2000);
    context.subscriptions.push({ dispose: () => clearInterval(timer) });
}
