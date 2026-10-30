import argparse
import errno
import hashlib
import json
import os
from pathlib import Path
import select
import signal
import socket
import stat
import subprocess
import sys
import time
import uuid


TOOLCHAIN = '/home/svakhreev/.rustup/toolchains/stable-x86_64-unknown-linux-gnu'
TOOLCHAIN_SHA256 = '9f3486a5f8dbe1859a242ca71018fd8b09e28cbb11d5da36f3a5e55eb716d3a7'
GIT_CONFIG = '[core]\nrepositoryformatversion = 0\nbare = false\nhooksPath = /dev/null\nfsmonitor = false\n'
BASES = ('d75ac44b027f702aa3af21c394021144f6ddd752',
         '2b5d29694d5b9b4e768f051b6baf0e04b2548dea')
ENGINE_CONFIG_SHA256 = '88d8399ce6e8f842ecb8ad1626162ce447485a800abba2318514d71de1b998c5'
OFFLINE_EXEC_SHA256 = '01a061af3164c67075a8da19c73acd74720901ea70f76b50780d72f6e92373b4'


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def namespaces():
    return {name: os.readlink('/proc/self/ns/' + name) for name in ('user', 'net', 'mnt', 'pid')}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def own_path(root, path):
    require(root.resolve() == root and path.is_relative_to(root), 'Noncanonical own path')
    cursor = root
    for part in path.relative_to(root).parts:
        cursor /= part
        require(not cursor.is_symlink(), 'Own path contains a symlink')
    require(path.resolve() == path, 'Own path escapes its canonical root')
    return path


def phase_git(root, *args):
    return subprocess.check_output(
        ['/usr/bin/git', '-c', 'core.fsmonitor=false', '-c', 'core.hooksPath=/dev/null', *args],
        cwd=root, env={'PATH': '/usr/bin:/bin', 'HOME': '/nonexistent', 'LANG': 'C.UTF-8',
                       'GIT_CONFIG_NOSYSTEM': '1', 'GIT_CONFIG_GLOBAL': '/dev/null',
                       'GIT_OPTIONAL_LOCKS': '0', 'GIT_TERMINAL_PROMPT': '0'},
        stderr=subprocess.PIPE, timeout=30).decode().strip()


def load_offline(root):
    if not os.environ.get('QUALIFICATION_PHASE'):
        import offline_exec

        return offline_exec
    import types

    path = Path('/qual-bootstrap/offline_exec.py')
    data = path.read_bytes()
    require(hashlib.sha256(data).hexdigest() == OFFLINE_EXEC_SHA256, 'Offline bootstrap changed')
    module = types.ModuleType('qualification_offline_exec')
    module.__file__ = str(path)
    exec(compile(data, str(path), 'exec'), module.__dict__)
    return module


def scan(source, hidden=()):
    counts = {'files': 0, 'directories': 0, 'symlinks': 0}
    deadline = time.monotonic() + 30
    for directory, directories, files in os.walk(source, followlinks=False):
        require(time.monotonic() < deadline, 'Mount scan exceeded its time bound')
        if Path(directory) == source:
            directories[:] = [name for name in directories if name not in hidden]
            files = [name for name in files if name not in hidden]
        for name in directories + files:
            path = Path(directory) / name
            mode = path.lstat().st_mode
            require(name != '.git', 'Nested Git metadata requires separate ingress review')
            if stat.S_ISREG(mode):
                counts['files'] += 1
            elif stat.S_ISDIR(mode):
                counts['directories'] += 1
            elif stat.S_ISLNK(mode):
                counts['symlinks'] += 1
            else:
                raise RuntimeError('Mount input contains a special file or socket')
    return counts


def regular_inventory(source):
    require(source.is_dir() and not source.is_symlink(), 'Input root must be a real directory')
    rows = []
    total = 0
    deadline = time.monotonic() + 60
    for directory, directories, files in os.walk(source, followlinks=False):
        for name in sorted(directories + files):
            require(time.monotonic() < deadline, 'Input inventory exceeded time bound')
            path = Path(directory) / name
            mode = path.lstat().st_mode
            relative = str(path.relative_to(source))
            require(not stat.S_ISLNK(mode), 'Symlink input requires separate review')
            if stat.S_ISDIR(mode):
                rows.append({'path': relative, 'kind': 'directory'})
            else:
                require(stat.S_ISREG(mode), 'Special input requires separate review')
                size = path.stat().st_size
                total += size
                require(size <= 67108864 and total <= 536870912, 'Input byte bound exceeded')
                rows.append({'path': relative, 'kind': 'file', 'bytes': size,
                             'sha256': digest(path)})
            require(len(rows) <= 24000, 'Input entry bound exceeded')
    return sorted(rows, key=lambda row: row['path'])


def toolchain_fingerprint(source):
    require(source.resolve() == source and source.is_dir(), 'Toolchain root must be canonical')
    fingerprint = hashlib.sha256()
    entries = 0
    total = 0
    deadline = time.monotonic() + 60
    for directory, directories, files in os.walk(source, followlinks=False):
        directories.sort()
        files.sort()
        for name in sorted(directories + files):
            require(time.monotonic() < deadline, 'Toolchain inventory exceeded time bound')
            path = Path(directory) / name
            info = path.lstat()
            relative = str(path.relative_to(source))
            if stat.S_ISDIR(info.st_mode):
                row = [relative, 'dir', stat.S_IMODE(info.st_mode)]
            else:
                require(stat.S_ISREG(info.st_mode), 'Nonregular toolchain input forbidden')
                total += info.st_size
                require(info.st_size <= 536870912 and total <= 2147483648,
                        'Toolchain input byte bound exceeded')
                row = [relative, 'file', info.st_size, stat.S_IMODE(info.st_mode), digest(path)]
            fingerprint.update((json.dumps(row, separators=(',', ':')) + '\n').encode())
            entries += 1
            require(entries <= 70000, 'Toolchain input entry bound exceeded')
    value = fingerprint.hexdigest()
    require(value == TOOLCHAIN_SHA256, 'Reviewed toolchain fingerprint changed')
    return {'sha256': value, 'entries': entries, 'file_bytes': total}


def phase_inputs(root):
    inputs = own_path(root, root / '.refact/qualification-r1/inputs')
    manifest = inputs / 'manifest.json'
    require(manifest.is_file() and manifest.stat().st_size <= 16777216, 'Manifest bound exceeded')
    raw = own_path(root, manifest).read_bytes()
    manifest_sha256 = hashlib.sha256(raw).hexdigest()
    data = json.loads(raw)
    head = phase_git(root, 'rev-parse', 'HEAD')
    tree = phase_git(root, 'rev-parse', 'HEAD^{tree}')
    require(data['head'] == head and data['tree'] == tree and data['bases'] == list(BASES),
            'Metadata provenance mismatch')
    require(regular_inventory(inputs / 'git') == data['git_inventory'], 'Git input changed')
    require((inputs / 'git-pointer').read_bytes() == b'gitdir: /qual-git\n', 'Git pointer mismatch')
    config = root / 'refact-agent/engine/.cargo/config.toml'
    require(digest(config) == data['engine_config_sha256'] == ENGINE_CONFIG_SHA256
            == digest(inputs / 'engine-config.toml'),
            'Reviewed engine config changed')
    require(digest(inputs / 'git-pointer') == data['git_pointer_sha256'], 'Git pointer hash changed')
    require((inputs / 'git/HEAD').read_text() == head + '\n', 'Detached Git HEAD mismatch')
    require((inputs / 'git/config').read_text() == GIT_CONFIG, 'Git config must be generated minimal')
    toolchain = toolchain_fingerprint(Path(TOOLCHAIN))
    require(toolchain == data['toolchain'], 'Toolchain provenance mismatch')
    state = root / '.refact/qualification-r1/state'
    for name in ('home', 'cargo', 'target'):
        path = own_path(root, state / name)
        path.mkdir(parents=True, exist_ok=True)
        require(path.resolve() == path and not list(path.iterdir()), 'Smoke state must start empty')
    identities = {name: [path.stat().st_dev, path.stat().st_ino]
                  for name, path in {'/qual-git': inputs / 'git',
                                     '/qual-toolchain': Path(TOOLCHAIN)}.items()}
    require(digest(manifest) == manifest_sha256, 'Validated manifest changed')
    return {'manifest_sha256': manifest_sha256, 'head': head, 'tree': tree, 'bases': list(BASES),
            'git_files': len(data['git_inventory']), 'toolchain': toolchain,
            'mount_identities': identities}


def phase_mounts(root, mounts):
    require(os.environ['QUALIFICATION_PHASE'] == 'smoke-rust', 'Unknown phase')
    require(Path('.git').read_bytes() == b'gitdir: /qual-git\n', 'Sanitized Git pointer missing')
    for path in (str(root / '.git'), '/qual-git', '/qual-toolchain', '/qual-inputs.json',
                 str(root / 'refact-agent/engine/.cargo/config.toml')):
        require('ro' in mounts[path]['options'], 'Phase ingress must remain read-only')
    for path in ('/qual-home', '/qual-cargo', '/qual-target'):
        require('rw' in mounts[path]['options'], 'Private mutable state missing')
    for name, identity in json.loads(os.environ['QUALIFICATION_INPUT_IDENTITIES']).items():
        path = Path(name)
        require([path.stat().st_dev, path.stat().st_ino] == identity, 'Ingress identity changed')
    manifest = Path('/qual-inputs.json')
    require(digest(manifest) == os.environ['QUALIFICATION_INPUT_MANIFEST_SHA256'],
            'Input manifest changed')
    data = json.loads(manifest.read_text())
    require(data['head'] == os.environ['QUALIFICATION_HEAD']
            and data['tree'] == os.environ['QUALIFICATION_TREE'] and data['bases'] == list(BASES),
            'Inner metadata provenance mismatch')
    require(regular_inventory(Path('/qual-git')) == data['git_inventory'], 'Git ingress changed')
    require(toolchain_fingerprint(Path('/qual-toolchain')) == data['toolchain'],
            'Toolchain ingress changed')
    require(digest(root / 'refact-agent/engine/.cargo/config.toml')
            == data['engine_config_sha256'] == ENGINE_CONFIG_SHA256,
            'Engine config ingress changed')
    metadata = Path('/qual-git')
    require((metadata / 'HEAD').read_text() == data['head'] + '\n', 'Inner detached HEAD mismatch')
    require(digest(root / '.git') == data['git_pointer_sha256'], 'Inner Git pointer mismatch')
    require((metadata / 'config').read_text() == GIT_CONFIG, 'Unexpected Git config')
    require(sorted(path.name for path in metadata.iterdir()) ==
            ['HEAD', 'config', 'index', 'objects', 'refs', 'shallow'], 'Unexpected Git metadata')
    require(not list((metadata / 'refs').iterdir()), 'Unexpected Git refs ingress')
    require(not (metadata / 'objects/info').exists(), 'Unexpected Git object indirection')
    require(not any(name in os.environ for name in (
        'GIT_DIR', 'GIT_WORK_TREE', 'GIT_INDEX_FILE', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER',
        'SCCACHE_SERVER_PORT', 'SCCACHE_DIR', 'RUSTUP_HOME')), 'Ambient execution override')


def smoke_commands():
    return [
        ['/qual-toolchain/bin/rustc', '--version', '--verbose'],
        ['/qual-toolchain/bin/cargo', '--version'],
        ['/usr/bin/python3', '-I', '-c', 'import sys; print(sys.version); assert sys.version_info >= (3, 10)'],
        ['/usr/bin/git', 'rev-parse', 'HEAD', 'HEAD^{tree}'],
        ['/usr/bin/git', 'fsck', '--full', '--strict', '--no-reflogs'],
        ['/usr/bin/python3', '-I', '-c',
         'import hashlib,json,subprocess; from pathlib import Path; '
         'm=json.loads(Path("/qual-inputs.json").read_text()); '
         'h=subprocess.check_output(["/usr/bin/git","rev-parse","HEAD","HEAD^{tree}"]); '
         'assert h.decode().splitlines()==[m["head"],m["tree"]]; '
         'p=subprocess.check_output(["/usr/bin/git","ls-files","-z"]); assert p; '
         'assert hashlib.sha256(p).hexdigest()==m["tracked_index_sha256"]; '
         'n=len(p.rstrip(b"\\0").split(b"\\0")); '
         'assert n==m["tracked_index_entries"]>0; print("tracked_index_entries",n)'],
    ]


def smoke_rust(root):
    signal.alarm(60)
    records = []
    for argv in smoke_commands():
        row = {'argv': argv, 'start_unix_ms': int(time.time() * 1000)}
        print(json.dumps({'smoke_command': row}), flush=True)
        result = subprocess.run(argv, cwd=root, stdin=subprocess.DEVNULL, timeout=20)
        row.update(exit_code=result.returncode, end_unix_ms=int(time.time() * 1000))
        records.append(row)
        print(json.dumps({'smoke_command_result': row}), flush=True)
        require(result.returncode == 0, 'Smoke command failed')
    signal.alarm(0)
    print(json.dumps({'passed': True, 'qualification': 'SMOKE_RUST_ONLY',
                      'commands': records, 'product_qualification': 'NOT_RUN'}), flush=True)


def command(root, host, token, home, phase=None, validated=None):
    argv = [
        '/usr/bin/bwrap', '--unshare-all', '--die-with-parent', '--new-session',
        '--cap-drop', 'ALL', '--cap-add', 'CAP_SETPCAP', '--clearenv',
        '--ro-bind', '/usr', '/usr',
        '--symlink', 'usr/bin', '/bin', '--symlink', 'usr/sbin', '/sbin',
        '--symlink', 'usr/lib', '/lib', '--symlink', 'usr/lib64', '/lib64',
        '--proc', '/proc', '--dev', '/dev', '--tmpfs', '/tmp', '--tmpfs', '/run',
        '--dir', '/etc', '--ro-bind', '/etc/ld.so.cache', '/etc/ld.so.cache',
        '--dir', '/home', '--dir', '/qual-home',
        '--bind', str(root), str(root), '--tmpfs', str(root / '.refact'),
        '--ro-bind', '/dev/null', str(root / '.git'),
    ]
    for relative in ('.ssh', '.config', '.cargo', '.gitconfig', 'refact-agent/engine/.cargo'):
        path = root / relative
        if path.exists() or path.is_symlink():
            require(not path.is_symlink(), 'Config mount masking requires a non-symlink input')
            argv += ['--tmpfs', str(path)] if path.is_dir() else ['--ro-bind', '/dev/null', str(path)]
    if phase is not None:
        inputs = root / '.refact/qualification-r1/inputs'
        state = root / '.refact/qualification-r1/state'
        argv += ['--ro-bind', str(inputs / 'git'), '/qual-git',
                 '--ro-bind', str(inputs / 'git-pointer'), str(root / '.git'),
                 '--ro-bind', str(inputs / 'manifest.json'), '/qual-inputs.json',
                 '--ro-bind', str(inputs / 'engine-config.toml'),
                 str(root / 'refact-agent/engine/.cargo/config.toml'),
                 '--ro-bind', TOOLCHAIN, '/qual-toolchain',
                 '--dir', '/qual-bootstrap', '--ro-bind', str(root / 'tools/dev/offline_exec.py'),
                 '/qual-bootstrap/offline_exec.py']
        for name in ('home', 'cargo', 'target'):
            argv += ['--bind', str(state / name), '/qual-' + name]
    argv += ['--chdir', str(root)]
    environment = {
        'PATH': '/usr/bin:/bin', 'HOME': '/qual-home', 'TMPDIR': '/tmp',
        'PYTHONDONTWRITEBYTECODE': '1', 'REFACT_RUN_LIVE_CHAT_TESTS': '0',
        'CARGO_NET_OFFLINE': 'true', 'npm_config_offline': 'true',
        'QUALIFICATION_HOST_NAMESPACES': json.dumps(host),
        'QUALIFICATION_TOKEN': token, 'QUALIFICATION_HOST_HOME': str(home),
        'QUALIFICATION_ROOT_IDENTITY': json.dumps([root.stat().st_dev, root.stat().st_ino]),
    }
    if phase is not None:
        environment.update({
            'PATH': '/usr/bin:/bin:/qual-toolchain/bin',
            'CARGO_HOME': '/qual-cargo', 'CARGO_TARGET_DIR': '/qual-target',
            'RUSTC': '/qual-toolchain/bin/rustc', 'RUSTDOC': '/qual-toolchain/bin/rustdoc',
            'CARGO_INCREMENTAL': '0', 'REFACT_SKIP_GUI_BUILD': '1',
            'XDG_CONFIG_HOME': '/qual-home/.config', 'XDG_CACHE_HOME': '/qual-home/.cache',
            'GIT_CONFIG_NOSYSTEM': '1', 'GIT_CONFIG_GLOBAL': '/dev/null',
            'GIT_OPTIONAL_LOCKS': '0', 'GIT_TERMINAL_PROMPT': '0',
            'LANG': 'C.UTF-8', 'LC_ALL': 'C.UTF-8',
            'QUALIFICATION_PHASE': phase,
            'QUALIFICATION_INPUT_MANIFEST_SHA256': validated['manifest_sha256'],
            'QUALIFICATION_HEAD': validated['head'], 'QUALIFICATION_TREE': validated['tree'],
            'QUALIFICATION_INPUT_IDENTITIES': json.dumps(validated['mount_identities']),
        })
    for key, value in environment.items():
        argv += ['--setenv', key, value]
    if phase:
        bootstrap = (
            'import hashlib,runpy,sys; from pathlib import Path; '
            'p="/qual-bootstrap/offline_exec.py"; '
            f'assert hashlib.sha256(Path(p).read_bytes()).hexdigest()=="{OFFLINE_EXEC_SHA256}"; '
            'sys.argv=[p,*sys.argv[1:]]; runpy.run_path(p,run_name="__main__")')
        return argv + ['--', '/usr/bin/python3', '-I', '-c', bootstrap,
                       '--inside', host['net'], '--', '/usr/bin/python3', '-I',
                       str(root / 'tools/dev/qualification_exec.py'), '--payload']
    return argv + ['--', '/usr/bin/python3', str(root / 'tools/dev/offline_exec.py'),
                   '--inside', host['net'], '--', '/usr/bin/python3',
                   str(root / 'tools/dev/qualification_exec.py'), '--payload']


def refused(endpoint):
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(2)
        try:
            client.connect(endpoint)
        except OSError as error:
            require(error.errno in (errno.ENOENT, errno.ECONNREFUSED, errno.EACCES),
                    'Outside owned socket refusal had an unexpected cause')
            return error.errno
    raise RuntimeError('Outside owned socket unexpectedly reachable')


def boundary(root, role):
    offline_exec = load_offline(root)

    token = os.environ['QUALIFICATION_TOKEN']
    host = json.loads(os.environ['QUALIFICATION_HOST_NAMESPACES'])
    current = namespaces()
    require(all(current[name] != host[name] for name in host), 'Namespace isolation failed')
    require(current['net'] == os.environ['REFACT_OFFLINE_NETNS'], 'Offline namespace changed')
    status = dict(line.split(':', 1) for line in Path('/proc/self/status').read_text().splitlines() if ':' in line)
    caps = {key: status[key].strip() for key in ('CapEff', 'CapPrm', 'CapInh', 'CapAmb', 'CapBnd')}
    require(all(int(value, 16) == 0 for value in caps.values()), 'Capabilities remain')
    require(status['NoNewPrivs'].strip() == '1', 'NoNewPrivs missing')
    offline_exec.validate_network()
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as control:
        require(offline_exec.loopback_flags(control) & 1, 'Loopback is not UP')
    require(Path.cwd() == root, 'Canonical checkout cwd mismatch')
    require([root.stat().st_dev, root.stat().st_ino] == json.loads(os.environ['QUALIFICATION_ROOT_IDENTITY']),
            'Own checkout bind identity mismatch')
    mounts = {}
    for line in Path('/proc/self/mountinfo').read_text().splitlines():
        left, right = line.split(' - ', 1)
        fields = left.split()
        mounts[fields[4]] = {'options': fields[5].split(','), 'type': right.split()[0]}
    require(mounts['/']['type'] == 'tmpfs', 'Host root must not be mounted')
    require('ro' in mounts['/usr']['options'], 'System mount must be read-only')
    require('rw' in mounts[str(root)]['options'], 'Own checkout must be writable')
    for path in ('/tmp', '/run', str(root / '.refact')):
        require(mounts[path]['type'] == 'tmpfs', 'Private tmpfs missing')
    require(mounts['/proc']['type'] == 'proc', 'Fresh proc mount missing')
    require(Path('/proc/1').is_dir() and os.getpid() != 1, 'PID namespace init missing')
    if os.environ.get('QUALIFICATION_PHASE'):
        phase_mounts(root, mounts)
    else:
        require(stat.S_ISCHR(Path('.git').stat().st_mode), 'Git metadata not hidden')
    require(not list(Path('.refact').iterdir()), 'Project metadata not hidden')
    require(sorted(path.name for path in Path('/etc').iterdir()) == ['ld.so.cache'], 'Unexpected host config ingress')
    require(not list(Path('/run').iterdir()), 'Host runtime ingress')
    host_home = Path(os.environ['QUALIFICATION_HOST_HOME'])
    for name in ('.ssh', '.config', '.cargo', '.gitconfig', '.refact'):
        require(not (host_home / name).exists(), 'Host credential/config path visible')
    if os.environ.get('QUALIFICATION_PHASE'):
        for path in (host_home / '.rustup/settings.toml',
                     host_home / 'projects/smc/refact/.git', host_home / '.gitconfig'):
            require(not path.exists(), 'Host toolchain settings or common Git metadata visible')
    require(not any(key.endswith(('_TOKEN', '_KEY', '_PASSWORD')) for key in os.environ if key != 'QUALIFICATION_TOKEN'),
            'Unexpected credential environment')
    marker = root / '.refact/outer-only-marker'
    require(not marker.exists(), 'Outer marker visible')
    try:
        marker.read_bytes()
    except FileNotFoundError:
        pass
    else:
        raise RuntimeError('Outer marker accessible')
    pathname_errno = refused('.refact/boundary.sock')
    abstract_errno = refused('\0qualification-' + token)
    proof = root / ('.qualification-rw-' + token + '-' + role)
    with proof.open('xb') as output:
        output.write(role.encode())
    require(proof.read_bytes() == role.encode(), 'Own write/read positive failed')
    return {'namespaces': current, 'capabilities': caps, 'no_new_privileges': 1,
            'only_loopback_up': True, 'IPv4_routes': [],
            'IPv6_routes_observed_not_certified': Path('/proc/net/ipv6_route').read_text(),
            'mounts': mounts, 'pid': os.getpid(), 'outside_pathname_errno': pathname_errno,
            'outside_abstract_errno': abstract_errno, 'own_rw_positive': proof.name,
            'hidden_metadata': True, 'host_config_absent': True}


def payload(root):
    phase = os.environ.get('QUALIFICATION_PHASE')
    if phase:
        import resource

        resource.setrlimit(resource.RLIMIT_FSIZE, (1048576, 1048576))
    signal.alarm(150 if phase else 15)
    parent = boundary(root, 'parent')
    child = None
    reaped = False
    endpoint = '/tmp/inside-owned.sock'
    try:
        with socket.socket(socket.AF_UNIX) as listener:
            listener.settimeout(70 if phase else 5)
            listener.bind(endpoint)
            listener.listen(1)
            read_end, write_end = os.pipe()
            child = os.fork()
            if child == 0:
                os.close(read_end)
                signal.alarm(60 if phase else 10)
                try:
                    report = boundary(root, 'fork')
                    with socket.socket(socket.AF_UNIX) as client:
                        client.settimeout(3)
                        client.connect(endpoint)
                        client.sendall(b'inside-owned-canary')
                    data = json.dumps(report).encode()
                    require(len(data) < 32768, 'Fork report exceeded bound')
                    with os.fdopen(write_end, 'wb') as output:
                        output.write(data)
                    os._exit(0)
                except BaseException:
                    os._exit(1)
            os.close(write_end)
            with os.fdopen(read_end, 'rb') as reports:
                with listener.accept()[0] as connection:
                    connection.settimeout(3)
                    require(connection.recv(64) == b'inside-owned-canary', 'Inner fork socket positive failed')
                require(select.select([reports], [], [], 5)[0], 'Fork report readiness timeout')
                report = json.loads(reports.read(32768))
            waited, status = os.waitpid(child, 0)
            reaped = True
            require(waited == child and status == 0, 'Fork canary did not exit cleanly')
            require(parent['namespaces'] == report['namespaces'], 'Fork containment changed')
        print(json.dumps({'passed': True, 'qualification': 'OWNED_BOUNDARY_ONLY',
                          'parent': parent, 'fork': report, 'inner_fork_socket_positive': True}), flush=True)
    finally:
        if child is not None and not reaped:
            try:
                os.kill(child, signal.SIGKILL)
            except ProcessLookupError:
                pass
            os.waitpid(child, 0)
        Path(endpoint).unlink(missing_ok=True)
        signal.alarm(0)
    if os.environ.get('QUALIFICATION_PHASE'):
        smoke_rust(root)


def canary(root, phase=None):
    require(Path.cwd() == root and not root.is_symlink(), 'Run from the canonical own checkout')
    evidence = root / '.refact/qualification-exec'
    if phase:
        own_path(root, evidence)
    evidence.mkdir(parents=True, exist_ok=True)
    name = phase or 'canary'
    record = evidence / (name + '.json')
    log = evidence / (name + '.log')
    require(not record.exists() and not log.exists(), 'Canary evidence exists; do not replay')
    clean = (phase_git(root, 'status', '--porcelain=v1') if phase else
             subprocess.check_output(['git', 'status', '--porcelain=v1'], text=True).strip())
    require(not clean,
            'Freeze a clean source head before the canary')
    token = uuid.uuid4().hex
    host = namespaces()
    inputs = phase_inputs(root) if phase is not None else None
    argv = command(root, host, token, Path.home(), phase, inputs)
    metadata = {
        'argv': argv, 'head': inputs['head'] if phase else subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
        'tree': inputs['tree'] if phase else subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], text=True).strip(),
        'source_sha256': {name: digest(root / 'tools/dev' / name) for name in ('offline_exec.py', 'qualification_exec.py')},
        'bwrap_version': subprocess.check_output(['/usr/bin/bwrap', '--version'], text=True).strip(),
        'host_namespaces': host, 'start_unix_ms': int(time.time() * 1000),
        'mount_input_scan': {'usr': scan(Path('/usr')), 'own': scan(root, ('.refact', '.git'))},
        'qualification': 'NOT_RUN',
    }
    if phase is not None:
        metadata.update(phase=phase, ingress=inputs, timeout_seconds=180,
                        log_limit_bytes=1048576, commands=smoke_commands(),
                        limitation='Read-only host binds do not prevent concurrent trusted-host mutation')
    marker = root / '.refact/outer-only-marker'
    endpoint = root / '.refact/boundary.sock'
    proofs = [root / ('.qualification-rw-' + token + '-' + role) for role in ('parent', 'fork')]
    require(not marker.exists() and not endpoint.exists(), 'Owned outer canary paths already exist')
    record.write_text(json.dumps(metadata, indent=2) + '\n')
    listeners = []
    try:
        with marker.open('xb') as output:
            output.write(b'owned-outer-only-canary')
        for address in ('.refact/boundary.sock', '\0qualification-' + token):
            listener = socket.socket(socket.AF_UNIX)
            listeners.append(listener)
            listener.settimeout(3)
            listener.bind(address)
            listener.listen(2)
            with socket.socket(socket.AF_UNIX) as positive:
                positive.settimeout(3)
                positive.connect(address)
                positive.sendall(b'outer-owned-baseline')
                with listener.accept()[0] as accepted:
                    accepted.settimeout(3)
                    require(accepted.recv(64) == b'outer-owned-baseline', 'Outer owned socket baseline failed')
        metadata['outer_pathname_and_abstract_positive'] = True
        with log.open('xb') as output:
            result = subprocess.run(argv, stdout=output, stderr=subprocess.STDOUT, env={},
                                    timeout=180 if phase else 30)
        metadata['exit_code'] = result.returncode
        unexpected = []
        for listener in listeners:
            listener.setblocking(False)
            try:
                connection, _ = listener.accept()
            except BlockingIOError:
                unexpected.append(0)
            else:
                connection.close()
                unexpected.append(1)
        metadata['outer_unexpected_connections'] = unexpected
        require(result.returncode == 0 and unexpected == [0, 0], 'Owned boundary canary failed')
        for role, proof in zip(('parent', 'fork'), proofs):
            require(proof.read_bytes() == role.encode(), 'Outer verification of own RW positive failed')
        report = json.loads(log.read_text().splitlines()[-1])
        require(report.get('passed') is True, 'Inner pass report missing')
        metadata['qualification'] = 'SMOKE_RUST_ONLY_PASS' if phase else 'OWNED_BOUNDARY_ONLY_PASS'
    except BaseException as error:
        metadata['qualification'] = 'REFUSED_STOP'
        metadata['error_type'] = type(error).__name__
        raise
    finally:
        for listener in listeners:
            listener.close()
        for path in (marker, endpoint, *proofs):
            path.unlink(missing_ok=True)
        metadata['end_unix_ms'] = int(time.time() * 1000)
        if log.exists():
            metadata.update(log_sha256=digest(log), log_bytes=log.stat().st_size)
        record.write_text(json.dumps(metadata, indent=2) + '\n')
        print(json.dumps(metadata, indent=2), flush=True)


def phase_canary(root, phase):
    evidence = own_path(root, root / '.refact/qualification-exec')
    evidence.mkdir(parents=True, exist_ok=True)
    record = own_path(root, evidence / (phase + '-preparation.json'))
    metadata = {'phase': phase, 'start_unix_ms': int(time.time() * 1000),
                'qualification': 'NOT_RUN', 'product_qualification': 'NOT_RUN'}
    with record.open('x') as output:
        output.write(json.dumps(metadata) + '\n')
    try:
        canary(root, phase)
        metadata['qualification'] = 'SMOKE_RUST_ONLY_PASS'
        return 0
    except (OSError, RuntimeError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        metadata.update(qualification='REFUSED_STOP', error_type=type(error).__name__)
        return 1
    finally:
        metadata.update(end_unix_ms=int(time.time() * 1000),
                        execution_record='SEPARATE' if (evidence / (phase + '.json')).exists() else 'NOT_RUN')
        record.write_text(json.dumps(metadata) + '\n')
        print(json.dumps(metadata), flush=True)


def main():
    parser = argparse.ArgumentParser()
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--canary', action='store_true')
    mode.add_argument('--phase', choices=('smoke-rust',))
    mode.add_argument('--payload', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    if args.canary:
        canary(root)
    elif args.phase:
        return phase_canary(root, args.phase)
    else:
        payload(root)


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, RuntimeError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print('Qualification execution refused: ' + type(error).__name__, file=sys.stderr)
        sys.exit(1)
