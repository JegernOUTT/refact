import fcntl
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys


def operation(stage, action):
    try:
        return action()
    except OSError as error:
        raise RuntimeError(f'{stage} failed (errno {error.errno})') from None


def namespace():
    return operation('network namespace read', lambda: os.readlink('/proc/self/ns/net'))


def validate_network():
    devices = operation('interface read', lambda: Path('/proc/net/dev').read_text())
    interfaces = {line.split(':')[0].strip() for line in devices.splitlines() if ':' in line}
    if interfaces != {'lo'}:
        raise RuntimeError('Offline namespace must contain only loopback')
    routes = operation('IPv4 route read', lambda: Path('/proc/net/route').read_text())
    if any(line.strip() and not line.startswith('Iface') for line in routes.splitlines()):
        raise RuntimeError('Offline namespace must not contain network routes')
    return interfaces


def loopback_flags(control):
    result = operation('loopback SIOCGIFFLAGS', lambda: fcntl.ioctl(
        control, 0x8913, struct.pack('16sH14x', b'lo', 0)))
    return struct.unpack('16sH14x', result)[1]


def inside(host_namespace, command):
    current = namespace()
    if current == host_namespace:
        raise RuntimeError('Offline execution requires a distinct network namespace')
    validate_network()
    with operation('loopback control socket', lambda: socket.socket(socket.AF_INET, socket.SOCK_DGRAM)) as control:
        flags = loopback_flags(control)
        if not flags & 1:
            operation('loopback SIOCSIFFLAGS UP', lambda: fcntl.ioctl(
                control, 0x8914, struct.pack('16sH14x', b'lo', flags | 1)))
        if not loopback_flags(control) & 1:
            raise RuntimeError('Offline loopback must be UP after bootstrap')
    interfaces = validate_network()
    if namespace() != current:
        raise RuntimeError('Offline namespace changed during bootstrap')
    os.environ['REFACT_OFFLINE_NETNS'] = current
    os.environ['REFACT_OFFLINE_HOST_NETNS'] = host_namespace
    os.environ['REFACT_RUN_LIVE_CHAT_TESTS'] = '0'
    for key in ('SSH_AUTH_SOCK', 'DBUS_SESSION_BUS_ADDRESS', 'DOCKER_HOST', 'DISPLAY', 'WAYLAND_DISPLAY'):
        os.environ.pop(key, None)
    print(json.dumps({'offline_namespace': current, 'host_namespace': host_namespace, 'interfaces': sorted(interfaces), 'command': command}), flush=True)
    operation('setpriv capability drop', lambda: os.execvp('setpriv', ['setpriv', '--no-new-privs', '--bounding-set=-all', '--inh-caps=-all', '--ambient-caps=-all', sys.executable, str(Path(__file__).resolve()), '--exec', current, '--', *command]))


def main():
    args = sys.argv[1:]
    if args[:1] == ['--exec']:
        if len(args) < 4 or args[2] != '--' or namespace() != args[1]:
            raise RuntimeError('Offline execution requires the original isolated namespace')
        raw_status = operation('capability status read', lambda: Path('/proc/self/status').read_text())
        status = dict(line.split(':', 1) for line in raw_status.splitlines() if ':' in line)
        capabilities = {key: status[key].strip() for key in ('CapEff', 'CapPrm', 'CapInh', 'CapAmb', 'CapBnd')}
        if any(int(value, 16) for value in capabilities.values()) or status['NoNewPrivs'].strip() != '1':
            raise RuntimeError('Offline execution requires all capabilities dropped')
        validate_network()
        with operation('final loopback control socket', lambda: socket.socket(socket.AF_INET, socket.SOCK_DGRAM)) as control:
            if not loopback_flags(control) & 1:
                raise RuntimeError('Offline loopback must remain UP')
        print(json.dumps({'offline_capabilities': capabilities, 'uid': os.getuid(), 'no_new_privileges': True}), flush=True)
        operation('offline command exec', lambda: os.execvp(args[3], args[3:]))
    elif args[:1] == ['--inside']:
        if len(args) < 4 or args[2] != '--':
            raise ValueError('Expected --inside <host namespace> -- <command>')
        inside(args[1], args[3:])
    else:
        if args[:1] == ['--']:
            args = args[1:]
        if not args:
            raise ValueError('An offline command is required')
        host = namespace()
        return operation('network namespace bootstrap', lambda: subprocess.call(['unshare', '--user', '--map-root-user', '--net', sys.executable, str(Path(__file__).resolve()), '--inside', host, '--', *args]))


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError) as error:
        print(f'Offline execution refused: {error}', file=sys.stderr)
        sys.exit(1)
