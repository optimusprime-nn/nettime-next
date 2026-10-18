import copy
import json
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch, Mock

from monitor import validate_config
from runtime_paths import SERVICE_NAME
from storage import Store
from takeover import CONTROLLERS, WindowsServices, activate, ownership_guard, read_journal, readiness, rollback


class FakeServices:
    def __init__(self, root):
        self.root = root
        self.services = {name: dict(name=name, exists=True, start_type=2 if name != 'W32Time' else 3,
            running=name != 'W32Time', account='NT AUTHORITY\\LocalService') for name in (*CONTROLLERS, SERVICE_NAME)}
        self.has_privilege = False
        self.fail_once = None
        self.calls = []

    def snapshot(self, name):
        return copy.deepcopy(self.services[name])

    def running(self, name):
        return self.services[name]['running']

    def configure(self, name, start_type=None, account=None):
        self.calls.append(('configure', name, start_type, account))
        if self.fail_once == name:
            self.fail_once = None
            raise OSError('Injected service configuration failure')
        if start_type is not None:
            self.services[name]['start_type'] = start_type
        if account is not None:
            self.services[name]['account'] = account

    def change(self, name, running):
        assert read_journal(self.root) is not None, 'Service changed before durable journal'
        self.calls.append(('change', name, running))
        self.services[name]['running'] = running

    def privilege(self, grant):
        previous = self.has_privilege
        if grant is not None:
            self.has_privilege = grant
        return previous


class TakeoverTests(unittest.TestCase):
    def test_virtual_account_requires_null_password_and_builtin_requires_empty(self):
        backend = WindowsServices()
        backend.api = Mock(SERVICE_NO_CHANGE=-1, SERVICE_CHANGE_CONFIG=2)
        backend.handle = Mock(return_value=Mock())
        backend.configure(SERVICE_NAME, account='NT SERVICE\\' + SERVICE_NAME)
        self.assertIsNone(backend.api.ChangeServiceConfig.call_args.args[9])
        backend.configure(SERVICE_NAME, account='NT AUTHORITY\\LocalService')
        self.assertEqual(backend.api.ChangeServiceConfig.call_args.args[9], '')

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.cfg = validate_config({'servers': [dict(host=f's{i}', source_group=f'g{i}') for i in range(3)]})
        (self.root / 'settings.json').write_text(json.dumps(self.cfg))
        self.store = Store(self.root / 'data/history.sqlite3')
        self.now = time.time()
        self.decision = dict(utc=self.now, selected='s0:123', confidence='test', synchronization=dict(ready=True, offset_ms=1,
                            sample_utc=self.now, explanation='Three groups agree'))
        self.store.save_cycle(1, [], self.decision, {}, self.now)
        self.backend = FakeServices(self.root)
        self.original = copy.deepcopy(self.backend.services)
        self.admin = patch('service_control.require_admin')
        self.admin.start()
        self.release_gate = patch('takeover.TAKEOVER_RELEASE_ENABLED', True)
        self.release_gate.start()

    def tearDown(self):
        self.release_gate.stop()
        self.admin.stop()
        self.temp.cleanup()

    def test_preview_release_gate_prevents_any_takeover_mutation(self):
        with patch('takeover.TAKEOVER_RELEASE_ENABLED', False):
            with self.assertRaisesRegex(RuntimeError, 'locked'):
                activate(self.root, self.backend)
        self.assertEqual(self.backend.calls, [])
        self.assertIsNone(read_journal(self.root))

    def test_activation_and_rollback_restore_exact_services_account_and_config(self):
        activate(self.root, self.backend)
        self.assertTrue(ownership_guard(self.root, self.backend))
        self.assertEqual(read_journal(self.root)['phase'], 'active')
        self.assertEqual(json.loads((self.root / 'settings.json').read_text())['clock']['mode'], 'slew')
        self.assertTrue(self.backend.has_privilege)
        self.assertTrue(all(not self.backend.running(name) for name in CONTROLLERS))
        rollback(self.root, self.backend)
        self.assertEqual(self.backend.services, self.original)
        self.assertEqual(json.loads((self.root / 'settings.json').read_text()), self.cfg)
        self.assertFalse(self.backend.has_privilege)
        self.assertEqual(read_journal(self.root)['phase'], 'rolled_back')
        rollback(self.root, self.backend)  # Idempotent.

    def test_partial_activation_failure_rolls_back_without_losing_history(self):
        self.backend.fail_once = 'W32Time'
        with self.assertRaises(OSError):
            activate(self.root, self.backend)
        self.assertEqual(self.backend.services, self.original)
        self.assertFalse(self.backend.has_privilege)
        self.assertEqual(self.store.latest_decision()[1]['selected'], 's0:123')
        self.assertEqual(read_journal(self.root)['phase'], 'rolled_back')

    def test_preexisting_privilege_is_retained_on_rollback(self):
        self.backend.has_privilege = True
        activate(self.root, self.backend)
        rollback(self.root, self.backend)
        self.assertTrue(self.backend.has_privilege)

    def test_bom_and_line_endings_are_preserved_by_rollback(self):
        path = self.root / 'settings.json'
        raw = b'\xef\xbb\xbf' + json.dumps(self.cfg, indent=2).replace('\n', '\r\n').encode('utf-8')
        path.write_bytes(raw)
        activate(self.root, self.backend)
        rollback(self.root, self.backend)
        self.assertEqual(path.read_bytes(), raw)

    def test_existing_transaction_is_never_overwritten(self):
        activate(self.root, self.backend)
        before = (self.root / 'clock-ownership.json').read_bytes()
        with self.assertRaises(RuntimeError):
            activate(self.root, self.backend)
        self.assertEqual(before, (self.root / 'clock-ownership.json').read_bytes())

    def test_conflict_revokes_guard_and_incomplete_rollback_can_be_retried(self):
        activate(self.root, self.backend)
        self.backend.services['W32Time']['running'] = True
        self.assertFalse(ownership_guard(self.root, self.backend))
        self.backend.fail_once = 'NetTimeSvc'
        with self.assertRaisesRegex(RuntimeError, 'Rollback incomplete'):
            rollback(self.root, self.backend)
        self.assertEqual(read_journal(self.root)['phase'], 'rolling_back')
        self.assertFalse(ownership_guard(self.root, self.backend))
        rollback(self.root, self.backend)
        self.assertEqual(self.backend.services, self.original)

    def test_stale_or_absent_evidence_or_custom_account_never_changes_services(self):
        for decision in (None, dict(self.decision, utc=self.now - 1000), dict(self.decision, synchronization=dict(ready=False))):
            self.assertFalse(readiness(self.cfg, decision, self.now)[0])
        with self.store.connect() as db:
            db.execute('DELETE FROM decisions')
        with self.assertRaisesRegex(RuntimeError, 'not ready'):
            activate(self.root, self.backend)
        self.assertEqual(self.backend.calls, [])
        self.store.save_cycle(1, [], self.decision, {}, self.now)
        self.backend.services[SERVICE_NAME]['account'] = 'custom\\account'
        with self.assertRaisesRegex(RuntimeError, 'custom accounts'):
            activate(self.root, self.backend)
        self.assertEqual(self.backend.calls, [])

    def test_general_settings_save_cannot_bypass_takeover(self):
        from service_control import apply_config
        attempted = copy.deepcopy(self.cfg)
        attempted['clock']['mode'] = 'slew'
        source = self.root / 'incoming.json'
        source.write_text(json.dumps(attempted))
        with patch('service_control.machine_root', return_value=self.root), patch('service_control.action') as action:
            with self.assertRaisesRegex(ValueError, 'ownership'):
                apply_config(source)
            action.assert_not_called()
