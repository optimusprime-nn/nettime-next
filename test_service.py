import json
import sqlite3
import tempfile
import unittest
import os
from pathlib import Path
from unittest.mock import patch

import app
import service_control
from monitor import Monitor, validate_config
from storage import Store

TEST_ROOT = Path(__file__).resolve().parent / "work" / "tests"
TEST_ROOT.mkdir(parents=True, exist_ok=True)


class ServiceContractTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=TEST_ROOT)
        self.root = Path(self.temp.name)
        self.cfg = validate_config({"servers": [{"host": "old.invalid", "security": "nts_required"}]})
        (self.root / "settings.json").write_text(json.dumps(self.cfg), encoding="utf-8")
        self.store = Store(self.root / "data" / "history.sqlite3")

    def tearDown(self):
        self.temp.cleanup()

    def test_desktop_reads_database_without_write_access(self):
        self.store.event("baseline", {})
        reader = Store(self.store.path, read_only=True)
        self.assertEqual(reader.latest_decision(), (0, None))
        self.assertEqual(reader.stats(), [])
        with self.assertRaises(sqlite3.OperationalError):
            reader.event("must_not_write", {})

    def test_service_view_does_not_start_another_collector(self):
        with patch("sys.argv", ["app.py", "--service-view"]), patch("app.machine_root", return_value=self.root), \
             patch("app.gui") as gui, patch("app.InstanceLock") as lock:
            app.main()
            self.assertTrue(gui.call_args.kwargs["service_view"])
            self.assertTrue(gui.call_args.args[1].read_only)
            lock.assert_not_called()

    def test_service_view_state_never_instantiates_a_collector(self):
        with patch("app.Monitor", side_effect=AssertionError("Collector must not be constructed")):
            view = app.view_state(self.cfg)
            self.assertEqual(view.peers[0].key, "old.invalid:123")

    def test_gui_startup_error_is_handled_without_second_exception(self):
        with patch("sys.argv", ["NetTimeNext.exe"]), patch("app.main", side_effect=OSError("Database unavailable")), \
             patch("tkinter.messagebox.showerror") as error:
            self.assertEqual(app.entrypoint(), 1)
            error.assert_called_once_with("NetTime Next", "Database unavailable")

    @unittest.skipUnless(os.name == "nt", "Windows filesystem permissions regression")
    def test_wal_migration_preserves_history_for_restricted_reader(self):
        import win32security as security
        from scripts.restricted_identity import restricted_identity
        protected = self.root / "protected"
        path = protected / "history.sqlite3"
        writer = Store(path)
        writer.event("retained", {"evidence": "must survive migration"})
        with sqlite3.connect(path) as db:
            self.assertEqual(db.execute("PRAGMA journal_mode=WAL").fetchone()[0], "wal")
        db.close()
        original = {p: security.GetFileSecurity(str(p), security.DACL_SECURITY_INFORMATION)
                    for p in (protected, path)}
        descriptor = security.ConvertStringSecurityDescriptorToSecurityDescriptor(
            "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;BU)", 1)

        def restrict():
            for p in (protected, path):
                security.SetFileSecurity(str(p), security.DACL_SECURITY_INFORMATION |
                    security.PROTECTED_DACL_SECURITY_INFORMATION, descriptor)

        def restore():
            for p in (protected, path):
                security.SetFileSecurity(str(p), security.DACL_SECURITY_INFORMATION |
                    security.PROTECTED_DACL_SECURITY_INFORMATION, original[p])

        try:
            restrict()
            with restricted_identity():
                with self.assertRaises(PermissionError):
                    (protected / "forbidden.tmp").write_text("must not be permitted")
                with self.assertRaises(sqlite3.OperationalError):
                    Store(path, read_only=True).next_cycle()
            restore()
            Store(path)  # Upgrade path: migrate, without removing recorded rows.
            restrict()
            with restricted_identity():
                reader = Store(path, read_only=True)
                self.assertEqual(reader.next_cycle(), 1)
                self.assertEqual(reader.latest_decision(), (0, None))
                with reader.connect() as db:
                    self.assertEqual(db.execute("PRAGMA journal_mode").fetchone()[0], "delete")
                    self.assertEqual(db.execute("SELECT kind FROM events").fetchone()[0], "retained")
                with self.assertRaises(PermissionError):
                    (protected / "forbidden.tmp").write_text("still forbidden")
        finally:
            restore()

    def test_reader_observes_new_commits_without_being_immutable(self):
        reader = Store(self.store.path, read_only=True)
        self.assertEqual(reader.latest_decision(), (0, None))
        decision = {"selected": "old.invalid:123", "confidence": "test", "servers": []}
        self.store.save_cycle(1, [], decision, {})
        self.assertEqual(reader.latest_decision()[1]["selected"], "old.invalid:123")

    def write_new(self):
        path = self.root / "incoming.json"
        path.write_text(json.dumps({"servers": [{"host": "new.invalid", "security": "nts_required"}],
                                    "retention_days": 10}), encoding="utf-8")
        return path

    def test_apply_settings_restarts_only_own_running_service(self):
        with patch("service_control.require_admin"), patch("service_control.machine_root", return_value=self.root), \
             patch("service_control.query_status", return_value="running"), patch("service_control.action") as action:
            service_control.apply_config(self.write_new())
            self.assertEqual([c.args[0] for c in action.call_args_list], ["stop", "start"])
            self.assertEqual(json.loads((self.root / "settings.json").read_text())["servers"][0]["host"], "new.invalid")
            with self.store.connect() as db:
                payload = json.loads(db.execute("SELECT payload FROM events WHERE kind='settings_applied'").fetchone()[0])
            self.assertEqual(payload["old"]["servers"][0]["host"], "old.invalid")

    def test_failed_start_restores_previous_settings(self):
        with patch("service_control.require_admin"), patch("service_control.machine_root", return_value=self.root), \
             patch("service_control.query_status", return_value="running"), \
             patch("service_control.action", side_effect=[None, RuntimeError("start failed"), None]) as action:
            with self.assertRaisesRegex(RuntimeError, "start failed"):
                service_control.apply_config(self.write_new())
            self.assertEqual(json.loads((self.root / "settings.json").read_text())["servers"][0]["host"], "old.invalid")
            self.assertEqual([c.args[0] for c in action.call_args_list], ["stop", "start", "start"])

    def test_stopped_service_stays_stopped_after_config_change(self):
        with patch("service_control.require_admin"), patch("service_control.machine_root", return_value=self.root), \
             patch("service_control.query_status", return_value="stopped"), patch("service_control.action") as action:
            service_control.apply_config(self.write_new())
            self.assertEqual([c.args[0] for c in action.call_args_list], ["stop"])

    def test_invalid_settings_never_stop_service(self):
        source = self.root / "invalid.json"
        source.write_text('{"servers":[],"retention_days":1}')
        with patch("service_control.require_admin"), patch("service_control.action") as action:
            with self.assertRaises(ValueError):
                service_control.apply_config(source)
            action.assert_not_called()

    def test_monitor_failure_reaches_service_host(self):
        monitor = Monitor(self.cfg, self.store)
        with patch.object(monitor, "collect", side_effect=OSError("disk failure")):
            with self.assertRaisesRegex(OSError, "disk failure"):
                monitor.run(raise_errors=True)

    def test_previous_selection_and_reach_survive_restart(self):
        decision = {"selected": "old.invalid:123", "confidence": "test", "servers": []}
        self.store.save_cycle(1, [], decision, {"old.invalid:123": {"reach": 255, "interval": 64}})
        monitor = Monitor(self.cfg, self.store)
        self.assertEqual(monitor.previous, "old.invalid:123")
        self.assertEqual(monitor.peers[0].reach, 255)


if __name__ == "__main__":
    unittest.main(verbosity=2)
