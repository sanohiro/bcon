"""Protocol/lifecycle tests without root, uinput, or touching the active VT."""
import importlib.util
import json
import multiprocessing
import os
from pathlib import Path
import socket
import tempfile
import time
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("input_helper", Path(__file__).with_name("input_helper.py"))
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)


class FakeGuard:
    reason = None
    tty = "tty2"

    def check(self):
        if self.reason:
            raise ValueError(self.reason)


class FakeKeyboard:
    def __init__(self):
        self.calls = []

    def tap(self, names, guard):
        guard.check()
        self.calls.append(names)

    def close(self):
        pass


class ProtocolTests(unittest.TestCase):
    def request(self, payload, guard=None, uid=None):
        keyboard = FakeKeyboard()
        a, b = socket.socketpair(socket.AF_UNIX, socket.SOCK_SEQPACKET)
        with a, b:
            a.sendall(json.dumps(payload).encode())
            response = h.handle(b, os.getuid() if uid is None else uid,
                                keyboard, guard or FakeGuard())
        return response, keyboard.calls

    def test_chord_and_modifier_order(self):
        response, calls = self.request({"op": "tap", "keys": "enter+ctrl+shift"})
        self.assertEqual(calls, [["LEFTCTRL", "LEFTSHIFT", "ENTER"]])
        self.assertTrue(response["ok"])

    def test_printscreen(self):
        self.assertEqual(h.chord("PRINTSCREEN"), ["SYSRQ"])

    def test_reject_dangerous_or_ambiguous_chords(self):
        for value in ("CTRL", "CTRL+CTRL+A", "A+B", "ALT+PRINTSCREEN",
                      "CTRL+ALT+F2", "CTRL+ALT+DELETE", "POWER", "", None):
            with self.subTest(value=value), self.assertRaises(ValueError):
                h.chord(value)

    def test_text_does_not_append_enter(self):
        response, calls = self.request({"op": "text", "text": "aA +!"})
        self.assertEqual(calls, [["A"], ["LEFTSHIFT", "A"], ["SPACE"],
                                 ["LEFTSHIFT", "EQUAL"], ["LEFTSHIFT", "1"]])
        self.assertEqual(response["taps"], 5)

    def test_full_printable_ascii(self):
        self.assertEqual(len(h.text_chords(''.join(chr(i) for i in range(32, 127)))), 95)

    def test_invalid_suffix_cannot_inject_prefix(self):
        keyboard = FakeKeyboard()
        a, b = socket.socketpair(socket.AF_UNIX, socket.SOCK_SEQPACKET)
        with a, b:
            a.sendall(json.dumps({"op": "text", "text": "abc\n"}).encode())
            with self.assertRaises(ValueError):
                h.handle(b, os.getuid(), keyboard, FakeGuard())
        self.assertEqual(keyboard.calls, [])

    def test_wrong_uid(self):
        response, calls = self.request({"op": "tap", "keys": "ENTER"}, uid=os.getuid() + 1)
        self.assertFalse(response["ok"])
        self.assertEqual(calls, [])

    def test_disarmed_rejects_tap_but_status_works(self):
        guard = FakeGuard()
        guard.reason = "VT changed"
        with self.assertRaises(ValueError):
            self.request({"op": "tap", "keys": "ENTER"}, guard)
        response, calls = self.request({"op": "status"}, guard)
        self.assertFalse(response["armed"])
        self.assertEqual(calls, [])

    def test_large_packet(self):
        a, b = socket.socketpair(socket.AF_UNIX, socket.SOCK_SEQPACKET)
        with a, b:
            a.sendall(b"x" * (h.MAX_PACKET + 1))
            with self.assertRaisesRegex(ValueError, "too large"):
                h.handle(b, os.getuid(), FakeKeyboard(), FakeGuard())

    def test_disconnected_client_sends_no_keys(self):
        a, b = socket.socketpair(socket.AF_UNIX, socket.SOCK_SEQPACKET)
        a.sendall(json.dumps({"op": "tap", "keys": "ENTER"}).encode())
        a.close()
        keyboard = FakeKeyboard()
        with b, self.assertRaisesRegex(ValueError, "disconnected"):
            h.handle(b, os.getuid(), keyboard, FakeGuard())
        self.assertEqual(keyboard.calls, [])

    def test_unknown_operation(self):
        with self.assertRaises(ValueError):
            self.request({"op": "exec", "command": "anything"})


class LifecycleTests(unittest.TestCase):
    def test_real_socket_client_server_and_shutdown_cleanup(self):
        context = multiprocessing.get_context("fork")
        stop = context.Event()
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory) / "helper.sock")

            def server():
                guard = FakeGuard()

                def check():
                    if stop.is_set():
                        raise KeyboardInterrupt

                guard.check = check
                try:
                    with patch.object(h, "Keyboard", return_value=FakeKeyboard()):
                        h.run_server(path, os.getuid(), -1, guard, os.getpid())
                except KeyboardInterrupt:
                    pass

            process = context.Process(target=server)
            process.start()
            try:
                deadline = time.monotonic() + 5
                while not Path(path).exists():
                    if time.monotonic() >= deadline or not process.is_alive():
                        self.fail("server did not bind")
                    time.sleep(0.01)
                self.assertTrue(h.client(path, {"op": "status"})["armed"])
                self.assertEqual(h.client(path, {"op": "tap", "keys": "PRINTSCREEN"}),
                                 {"ok": True, "taps": 1})
                self.assertFalse(h.client(path, {"op": "unknown"})["ok"])
            finally:
                stop.set()
                process.join(5)
                if process.is_alive():
                    process.kill()
                    process.join()
            self.assertEqual(process.exitcode, 0)
            self.assertFalse(Path(path).exists())

    def test_client_rejects_world_accessible_socket(self):
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory) / "helper.sock")
            with socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET) as server:
                server.bind(path)
                os.chmod(path, 0o666)
                with self.assertRaisesRegex(ValueError, "0600"):
                    h.client(path, {"op": "status"})

    def test_process_exit_disarms_guard(self):
        pidfd = os.pidfd_open(os.getpid())
        try:
            guard = h.Guard("tty2", pidfd)
            with patch.object(h.select, "select", return_value=([pidfd], [], [])):
                with self.assertRaisesRegex(ValueError, "exited"):
                    guard.check()
        finally:
            os.close(pidfd)

    def test_vt_change_latches_even_if_restored(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "active"
            path.write_text("tty2\n")
            pidfd = os.pidfd_open(os.getpid())
            try:
                guard = h.Guard("tty2", pidfd, path)
                guard.check()
                path.write_text("tty1\n")
                with self.assertRaises(ValueError):
                    guard.check()
                path.write_text("tty2\n")
                with self.assertRaises(ValueError):
                    guard.check()
            finally:
                os.close(pidfd)

    def test_keys_released_when_guard_fails_mid_chord(self):
        keyboard = object.__new__(h.Keyboard)
        keyboard.codes = {"LEFTCTRL": 29, "ENTER": 28}
        keyboard.held = []
        events = []
        keyboard.event = lambda code, value: events.append((code, value))
        guard = FakeGuard()
        calls = 0

        def check():
            nonlocal calls
            calls += 1
            if calls == 2:
                raise ValueError("VT switched")

        guard.check = check
        with self.assertRaises(ValueError):
            keyboard.tap(["LEFTCTRL", "ENTER"], guard)
        self.assertEqual(events, [(29, 1), (29, 0)])
        self.assertEqual(keyboard.held, [])

    def test_release_attempts_remaining_keys_after_io_error(self):
        keyboard = object.__new__(h.Keyboard)
        keyboard.held = [29, 28]
        events = []

        def event(code, value):
            events.append((code, value))
            if code == 28:
                raise OSError("write failed")

        keyboard.event = event
        with self.assertRaises(OSError):
            keyboard.release()
        self.assertEqual(events, [(28, 0), (29, 0)])
        self.assertEqual(keyboard.held, [])

    def test_existing_socket_is_never_replaced(self):
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory) / "helper.sock")
            with socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET) as existing:
                existing.bind(path)
                inode = os.stat(path).st_ino
                with self.assertRaises(OSError):
                    h.run_server(path, os.getuid(), -1, FakeGuard(), 1)
                self.assertEqual(os.stat(path).st_ino, inode)


if __name__ == "__main__":
    unittest.main()
