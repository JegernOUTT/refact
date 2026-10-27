import fcntl
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys


def namespace():
    return os.readlink('/proc/self/ns/net')


def inside(host_namespace, command):
    current = namespace()
    if current == host_namespace:
        raise RuntimeError('Offline execution requires a distinct network namespace')
    interfaces = {line.split(':')[0].strip() for line in Path('/proc/net/dev').read_text().splitlines() if ':' in line}
    if interfaces != {'lo'}:
        raise RuntimeError('Offline namespace must contain only loopback')
    if any(line.strip() and not line.startswith('Iface') for line in Path('/proc/net/route').read_text().splitlines()):
        raise RuntimeError('Offline namespace must not contain network routes')
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as control:
        request = struct.pack('16sH14x', b'lo', 0)
        flags = struct.unpack('16sH14x', fcntl.ioctl(control, 0x8913, request))[1]
        fcntl.ioctl(control, 0x8914, struct.pack('16sH14x', b'lo', flags | 1))
    os.environ['REFACT_OFFLINE_NETNS'] = current
    os.environ['REFACT_OFFLINE_HOST_NETNS'] = host_namespace
    os.environ['REFACT_RUN_LIVE_CHAT_TESTS'] = '0'
    for key in ('SSH_AUTH_SOCK', 'DBUS_SESSION_BUS_ADDRESS', 'DOCKER_HOST', 'DISPLAY', 'WAYLAND_DISPLAY'):
        os.environ.pop(key, None)
    print(json.dumps({'offline_namespace': current, 'host_namespace': host_namespace, 'interfaces': sorted(interfaces), 'command': command}), flush=True)
    os.execvp('setpriv', ['setpriv', '--no-new-privs', '--bounding-set=-all', '--inh-caps=-all', '--ambient-caps=-all', sys.executable, str(Path(__file__).resolve()), '--exec', current, '--', *command])


def main():
    args = sys.argv[1:]
    if args[:1] == ['--exec']:
        if len(args) < 4 or args[2] != '--' or namespace() != args[1]:
            raise RuntimeError('Offline execution requires the original isolated namespace')
        status = dict(line.split(':', 1) for line in Path('/proc/self/status').read_text().splitlines() if ':' in line)
        capabilities = {key: status[key].strip() for key in ('CapEff', 'CapPrm', 'CapInh', 'CapAmb', 'CapBnd')}
        if any(int(value, 16) for value in capabilities.values()) or status['NoNewPrivs'].strip() != '1':
            raise RuntimeError('Offline execution requires all capabilities dropped')
        print(json.dumps({'offline_capabilities': capabilities, 'uid': os.getuid(), 'no_new_privileges': True}), flush=True)
        os.execvp(args[3], args[3:])
    elif args[:1] == ['--inside']:
        if len(args) < 4 or args[2] != '--':
            raise ValueError('Expected --inside <host namespace> -- <command>')
        inside(args[1], args[3:])
    else:
        if args[:1] == ['--']:
            args = args[1:]
        if not args:
            raise ValueError('An offline command is required')
        return subprocess.call(['unshare', '--user', '--map-root-user', '--net', sys.executable, str(Path(__file__).resolve()), '--inside', namespace(), '--', *args])


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError) as error:
        print(f'Offline execution refused: {error}', file=sys.stderr)
        sys.exit(1)
