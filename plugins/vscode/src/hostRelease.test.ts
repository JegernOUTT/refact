/* eslint-disable @typescript-eslint/naming-convention */
import * as assert from 'assert';
import { HostReleaseOwner, HostReleaseRequest, HostReleaseReceipt, ReleaseClaim, ReleaseHost, ReleaseTransport, manifestDigest, sha256, verifyVsix, crc32 } from './hostRelease';

// Stored ZIP fixture, parsed by the real verifier (not a mocked verifier).
function zip(files: Record<string, string>): Buffer {
    const local: Buffer[] = [], central: Buffer[] = []; let offset = 0;
    for (const [name, text] of Object.entries(files)) {
        const data = Buffer.from(text), filename = Buffer.from(name);
        const header = Buffer.alloc(30); header.writeUInt32LE(0x04034b50, 0);
        header.writeUInt32LE(data.length, 18); header.writeUInt32LE(data.length, 22); header.writeUInt16LE(filename.length, 26);
        const entry = Buffer.alloc(46); entry.writeUInt32LE(0x02014b50, 0);
        entry.writeUInt32LE(data.length, 20); entry.writeUInt32LE(data.length, 24); entry.writeUInt16LE(filename.length, 28); entry.writeUInt32LE(offset, 42);
        header.writeUInt32LE(crc32(data), 14); entry.writeUInt32LE(crc32(data), 16);
        local.push(header, filename, data); central.push(entry, filename); offset += header.length + filename.length + data.length;
    }
    const dir = Buffer.concat(central), end = Buffer.alloc(22); end.writeUInt32LE(0x06054b50, 0);
    end.writeUInt16LE(Object.keys(files).length, 8); end.writeUInt16LE(Object.keys(files).length, 10);
    end.writeUInt32LE(dir.length, 12); end.writeUInt32LE(offset, 16);
    return Buffer.concat([...local, dir, end]);
}
function fixture() {
    const files = { 'out/main.js': sha256('new code') };
    const bytes = zip({ 'extension/package.json': JSON.stringify({ name: 'codify', publisher: 'smallcloud', version: '2.0.0' }), 'extension/out/main.js': 'new code' });
    const key = { prepared_hash: sha256('prepared'), grant_id: 'grant', project_id: 'project', host_session_id: 'activation-1' };
    const before = { manifest_digest: sha256('old manifest'), version: '1.0.0' };
    const request: HostReleaseRequest = { ...key, schema_version: 1, target: 'vscode', operation: 'install', candidate_id: 'candidate', candidate_digest: sha256('candidate'), proof_id: 'proof', artifact_digest: sha256(bytes), manifest_digest: manifestDigest(files), package_version: '2.0.0', expected_installed_manifest_digest: before.manifest_digest, expected_running_manifest_digest: before.manifest_digest, files };
    let installed = { ...before }, calls = 0, confirms = 0, accepted = true, installerError = false, reportError = false;
    const claims: ReleaseClaim[] = [], reports: HostReleaseReceipt[] = [];
    const host: ReleaseHost = {
        running: { ...before, host_session_id: key.host_session_id },
        installed: async () => ({ ...installed }),
        confirm: async () => { confirms++; return accepted; },
        stage: async (staged) => {
            assert.deepStrictEqual(staged, bytes);
            return { install: async () => {
                assert.strictEqual(claims[0].receipt.status, 'outcome_unknown');
                calls++; installed = { manifest_digest: request.manifest_digest, version: request.package_version };
                if (installerError) { throw new Error('lost host reply'); }
            }, dispose: async () => undefined };
        },
        claims: async () => structuredClone(claims),
        claim: async c => { if (claims.length) { return false; } claims.push(structuredClone(c)); return true; },
        save: async c => { claims[0] = structuredClone(c); }
    };
    const transport: ReleaseTransport = {
        fetch: async () => ({ request, bytes }),
        report: async (_key, receipt) => { if (reportError) { throw new Error('lost daemon reply'); } reports.push(structuredClone(receipt)); },
        reconcile: async (_key, observed, running) => { assert.deepStrictEqual(observed, installed); assert.strictEqual(running.host_session_id, 'activation-2'); }
    };
    return { key, bytes, request, before, host, transport, claims, reports,
        calls: () => calls, confirms: () => confirms,
        cancel: () => { accepted = false; }, installerError: () => { installerError = true; }, reportError: () => { reportError = true; } };
}
// Execute real native registration, HTTP transport, scratch claim journal and
// activation snapshot. Only VS Code APIs (especially installation) are injected.
async function nativeReachability(): Promise<void> {
    const fs = await import('fs'), os = await import('os'), path = await import('path'), http = await import('http');
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'refact-native-release-'));
    const extensionPath = path.join(root, 'extension'), storage = path.join(root, 'storage');
    fs.mkdirSync(path.join(extensionPath, 'out'), { recursive: true });
    fs.mkdirSync(storage);
    fs.writeFileSync(path.join(extensionPath, 'out/main.js'), 'old code');
    fs.writeFileSync(path.join(extensionPath, 'package.json'), '{}');
    const f = fixture(), callbacks = new Map<string, (...args: unknown[]) => unknown>();
    const subscriptions: Array<{ dispose(): unknown }> = [], secrets = new Map<string, string>();
    const replies: Array<string | undefined> = [], requests: Array<{ endpoint: string; body: Record<string, unknown> }> = [];
    let installs = 0, copied = '', currentVersion = '1.0.0';
    const vscode = {
        commands: {
            registerCommand: (id: string, callback: (...args: unknown[]) => unknown) => { callbacks.set(id, callback); return { dispose: () => callbacks.delete(id) }; },
            executeCommand: async (id: string, uri: { fsPath: string }) => {
                assert.strictEqual(id, 'workbench.extensions.installExtension');
                assert.deepStrictEqual(fs.readFileSync(uri.fsPath), f.bytes);
                const ledger = fs.readdirSync(path.join(storage, 'host-release-claims')).find(n => n.endsWith('.json'))!;
                assert.strictEqual(JSON.parse(fs.readFileSync(path.join(storage, 'host-release-claims', ledger), 'utf8')).receipt.status, 'outcome_unknown');
                installs++;
                fs.writeFileSync(path.join(extensionPath, 'out/main.js'), 'new code');
                currentVersion = '2.0.0';
            }
        },
        window: { showInputBox: async () => replies.shift(), showWarningMessage: async () => 'Install verified package', showInformationMessage: async () => 'Copy identity' },
        extensions: { getExtension: () => ({ extensionPath, packageJSON: { version: currentVersion } }) },
        Uri: { file: (fsPath: string) => ({ fsPath }) },
        env: { clipboard: { writeText: async (value: string) => { copied = value; } } }
    };
    // Native adapter imports vscode at load time; substitute only that host API.
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const modules = require('module') as { _load: (name: string, ...args: unknown[]) => unknown };
    const oldLoad = modules._load;
    modules._load = function (name: string, ...args: unknown[]): unknown { return name === 'vscode' ? vscode : oldLoad.call(this, name, ...args); };
    let native: typeof import('./nativeHostRelease');
    try {
        // eslint-disable-next-line @typescript-eslint/no-var-requires
        native = require('./nativeHostRelease') as typeof import('./nativeHostRelease');
    } finally { modules._load = oldLoad; }
    const server = http.createServer((req, res) => {
        const chunks: Buffer[] = [];
        req.on('data', (chunk: Buffer) => chunks.push(chunk));
        req.on('end', () => {
            const body = JSON.parse(Buffer.concat(chunks).toString()) as Record<string, unknown>;
            const endpoint = req.url!.split('/').pop()!;
            requests.push({ endpoint, body });
            if (req.headers.authorization !== 'Basic ' + Buffer.from('user:password').toString('base64')) { res.writeHead(401).end(); return; }
            res.setHeader('Content-Type', 'application/json');
            res.end(JSON.stringify(endpoint === 'artifact' ? { request: f.request, artifact_base64: f.bytes.toString('base64') } : {}));
        });
    });
    const oldDaemon = global.rust_binary_blob;
    try {
        await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
        const address = server.address() as import('net').AddressInfo;
        global.rust_binary_blob = { port: address.port, project_id: f.key.project_id } as typeof global.rust_binary_blob;
        const context = () => ({ extensionPath, extension: { id: 'smallcloud.codify', packageJSON: { version: currentVersion } }, globalStorageUri: { fsPath: storage }, subscriptions, secrets: { get: async (key: string) => secrets.get(key), store: async (key: string, value: string) => { secrets.set(key, value); } } }) as unknown as import('vscode').ExtensionContext;
        native.registerNativeHostRelease(context());
        const manifest = JSON.parse(fs.readFileSync(path.join(__dirname, '../package.json'), 'utf8')) as { contributes: { commands: Array<{ command: string }> } };
        for (const id of ['refactaicmd.hostRelease', 'refactaicmd.hostReleaseIdentity', 'refactaicmd.hostReleaseReconcile']) {
            assert.ok(callbacks.has(id), `${id} is registered`);
            assert.ok(manifest.contributes.commands.some(c => c.command === id), `${id} is palette reachable`);
        }
        // No automatic timer/network action is needed to prove explicit user command reachability.
        for (const subscription of subscriptions.splice(3)) { subscription.dispose(); }
        const identity = await callbacks.get('refactaicmd.hostReleaseIdentity')!() as { manifest_digest: string; host_session_id: string };
        assert.deepStrictEqual(JSON.parse(copied), identity);
        f.key.host_session_id = identity.host_session_id;
        f.request.host_session_id = identity.host_session_id;
        f.request.expected_installed_manifest_digest = identity.manifest_digest;
        f.request.expected_running_manifest_digest = identity.manifest_digest;
        const release = callbacks.get('refactaicmd.hostRelease')!;
        replies.push(undefined);
        await release(); // palette cancellation: no secret, request, claim or effect
        assert.strictEqual(requests.length, 0); assert.strictEqual(installs, 0); assert.strictEqual(secrets.size, 0);
        await assert.rejects(async () => release({ ...f.key, project_id: 'foreign' }));
        await assert.rejects(async () => release({ ...f.key, host_session_id: 'foreign' }));
        replies.push(f.key.prepared_hash, f.key.grant_id, 'user', 'password');
        const receipt = await release() as HostReleaseReceipt;
        assert.strictEqual(receipt.status, 'installer_returned');
        assert.strictEqual(receipt.installed?.manifest_digest, f.request.manifest_digest);
        assert.strictEqual(receipt.running?.manifest_digest, identity.manifest_digest);
        assert.strictEqual(installs, 1);
        const immutable = JSON.stringify(receipt);
        replies.push('user', 'password');
        await assert.rejects(async () => release(f.key));
        assert.strictEqual(installs, 1);
        // Reopen the same durable journal, taking an actual fresh activation snapshot
        // from scratch installed bytes. No reload/install command is dispatched.
        for (const subscription of subscriptions.splice(0)) { subscription.dispose(); }
        callbacks.clear();
        native.registerNativeHostRelease(context());
        for (const subscription of subscriptions.splice(3)) { subscription.dispose(); }
        await callbacks.get('refactaicmd.hostReleaseReconcile')!();
        const observations = requests.filter(r => r.endpoint === 'observe');
        assert.strictEqual(observations.length, 1);
        const observation = observations[0].body;
        assert.strictEqual(observation.schema_version, 1);
        assert.strictEqual(observation.host_session_id, identity.host_session_id);
        assert.strictEqual(observation.artifact_digest, f.request.artifact_digest);
        assert.deepStrictEqual(observation.installed, receipt.installed);
        const running = observation.running as { host_session_id: string; manifest_digest: string; version: string };
        assert.notStrictEqual(running.host_session_id, identity.host_session_id);
        assert.strictEqual(running.manifest_digest, f.request.manifest_digest);
        assert.strictEqual(running.version, f.request.package_version);
        assert.strictEqual(JSON.stringify(requests.filter(r => r.endpoint === 'receipt').pop()!.body), immutable);
        assert.strictEqual(installs, 1);
    } finally {
        for (const subscription of subscriptions) { subscription.dispose(); }
        global.rust_binary_blob = oldDaemon;
        await new Promise<void>(resolve => server.close(() => resolve()));
        fs.rmSync(root, { recursive: true, force: true });
    }
}

async function main(): Promise<void> {
    await nativeReachability();
    for (const mutate of [
        (f: ReturnType<typeof fixture>) => { f.request.artifact_digest = sha256('wrong'); },
        (f: ReturnType<typeof fixture>) => { f.request.target = 'jetbrains'; },
        (f: ReturnType<typeof fixture>) => { f.request.host_session_id = 'foreign'; },
        (f: ReturnType<typeof fixture>) => { f.request.expected_installed_manifest_digest = sha256('foreign'); },
        (f: ReturnType<typeof fixture>) => { f.request.expected_running_manifest_digest = sha256('foreign'); },
        (f: ReturnType<typeof fixture>) => { f.request.package_version = '9.0.0'; },
        (f: ReturnType<typeof fixture>) => { f.request.project_id = 'foreign'; },
        (f: ReturnType<typeof fixture>) => { f.request.files['out/main.js'] = sha256('foreign'); }
    ]) {
        const f = fixture(); mutate(f);
        await assert.rejects(new HostReleaseOwner(f.host, f.transport).run(f.key));
        assert.strictEqual(f.calls(), 0); assert.strictEqual(f.confirms(), 0);
    }
    {
        const f = fixture(); f.cancel();
        const result = await new HostReleaseOwner(f.host, f.transport).run(f.key);
        assert.strictEqual(result.status, 'declined'); assert.strictEqual(f.calls(), 0);
    }
    {
        const f = fixture(), owner = new HostReleaseOwner(f.host, f.transport);
        const pending = owner.run(f.key);
        await assert.rejects(owner.run(f.key));
        const result = await pending;
        assert.strictEqual(result.status, 'installer_returned');
        assert.strictEqual(result.installed?.manifest_digest, f.request.manifest_digest);
        assert.strictEqual(result.running?.manifest_digest, f.before.manifest_digest);
        assert.notStrictEqual(result.running?.manifest_digest, result.installed?.manifest_digest);
        await assert.rejects(new HostReleaseOwner(f.host, f.transport).run(f.key));
        const activated = { ...f.host, running: { ...result.installed!, host_session_id: 'activation-2' } };
        await new HostReleaseOwner(activated, f.transport).reconcile();
        assert.strictEqual(f.calls(), 1);
    }
    {
        const f = fixture(); f.installerError();
        const receipt = await new HostReleaseOwner(f.host, f.transport).run(f.key);
        assert.strictEqual(receipt.status, 'outcome_unknown');
        assert.strictEqual(receipt.installed?.manifest_digest, f.request.manifest_digest);
        await assert.rejects(new HostReleaseOwner(f.host, f.transport).run(f.key));
        assert.strictEqual(f.calls(), 1);
    }
    {
        const f = fixture(); f.reportError();
        await assert.rejects(new HostReleaseOwner(f.host, f.transport).run(f.key));
        await assert.rejects(new HostReleaseOwner(f.host, f.transport).run(f.key));
        assert.strictEqual(f.calls(), 1); assert.strictEqual(f.claims.length, 1);
    }
    {
        const f = fixture(); f.claims.push({ key: f.key, receipt: { schema_version: 1, ...f.key, artifact_digest: f.request.artifact_digest, status: 'outcome_unknown', installed: null, running: null } });
        await assert.rejects(new HostReleaseOwner(f.host, f.transport).run(f.key)); assert.strictEqual(f.calls(), 0);
    }
    {
        const f = fixture(); verifyVsix(f.bytes, f.request);
        const bad = zip({ 'extension/../outside': 'bad' });
        assert.throws(() => verifyVsix(bad, { ...f.request, artifact_digest: sha256(bad) }));
        assert.strictEqual(manifestDigest({ '10': sha256('a'), '2': sha256('b') }), sha256('{"10":"' + sha256('a') + '","2":"' + sha256('b') + '"}'));
    }
    {
        const f = fixture();
        await assert.rejects(new HostReleaseOwner(f.host, f.transport).run({ ...f.key, host_session_id: 'foreign' }));
        await assert.rejects(new HostReleaseOwner(f.host, f.transport).run({ ...f.key, path: '/tmp/untrusted.vsix' } as typeof f.key));
        assert.strictEqual(f.calls(), 0);
    }
    {
        const f = fixture(); f.request.operation = 'recover';
        const receipt = await new HostReleaseOwner(f.host, f.transport).run(f.key);
        assert.strictEqual(receipt.status, 'installer_returned'); assert.strictEqual(f.confirms(), 1);
    }
    {
        const f = fixture(); let observations = 0;
        f.host.installed = async () => ({ ...f.before, manifest_digest: ++observations > 1 ? sha256('changed after confirmation') : f.before.manifest_digest });
        await assert.rejects(new HostReleaseOwner(f.host, f.transport).run(f.key)); assert.strictEqual(f.calls(), 0);
    }
    {
        const f = fixture(); f.host.claim = async () => { throw new Error('persistence unavailable'); };
        await assert.rejects(new HostReleaseOwner(f.host, f.transport).run(f.key)); assert.strictEqual(f.calls(), 0);
    }
    console.log('hostRelease: verification, cancellation, durable claim/no-replay, unknown outcome, activation identity tests passed');
}
void main().catch(error => { console.error(error); process.exitCode = 1; });
