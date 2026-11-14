import hashlib
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import unittest
from unittest.mock import Mock

from owned_engine import Engine, ephemeral_port, owns_listener, verified_copy


class OwnedEngineTests(unittest.TestCase):
    def test_copy_pins_identity_and_rejects_changed_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "source"
            binary.write_bytes(b"first")
            expected = hashlib.sha256(b"first").hexdigest()
            copied = verified_copy(binary, expected, root / "copy")
            binary.write_bytes(b"second")
            self.assertEqual(copied.read_bytes(), b"first")
            with self.assertRaisesRegex(AssertionError, "differs"):
                verified_copy(binary, expected, root / "wrong")

    def test_stop_only_owned_process_and_close_log(self):
        engine = Engine(Path("unused"), Path("unused"), Path("unused"), Path("unused"))
        process = Mock()
        process.poll.side_effect = [None, 0]
        process.wait.side_effect = [subprocess.TimeoutExpired("owned", 15), 0]
        engine.process = process
        engine.log = Mock()
        engine.stop()
        process.terminate.assert_called_once_with()
        process.kill.assert_called_once_with()
        self.assertEqual(process.wait.call_count, 2)
        engine.log.close.assert_called_once_with()

    def test_terminal_process_is_not_signalled(self):
        engine = Engine(Path("unused"), Path("unused"), Path("unused"), Path("unused"))
        engine.process = Mock()
        engine.process.poll.return_value = 0
        engine.stop()
        engine.process.terminate.assert_not_called()
        engine.process.kill.assert_not_called()

    def test_listener_identity_rejects_other_pid(self):
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen()
            port = listener.getsockname()[1]
            self.assertTrue(owns_listener(os.getpid(), port))
            self.assertFalse(owns_listener(1, port))

    def test_ports_are_loopback_ephemeral(self):
        port = ephemeral_port()
        self.assertGreater(port, 0)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", port))


if __name__ == "__main__":
    unittest.main()
