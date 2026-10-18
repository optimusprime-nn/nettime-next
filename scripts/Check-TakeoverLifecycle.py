"""Ephemeral Windows CI only: real SCM ownership change, no clock samples.

The installer fixture uses a reserved .invalid NTS host. Inject a readiness
record to exercise activation, virtual-account privilege and rollback without
ever supplying a usable sample to the real controller. Never run on a user's PC.
"""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from monitor import load_config
from runtime_paths import SERVICE_NAME, machine_root
from storage import Store
from takeover import CONTROLLERS, WindowsServices, read_journal
from unittest.mock import patch


def main():
    if os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted':
        raise RuntimeError('This destructive integration fixture is restricted to disposable GitHub-hosted runners')
    root = machine_root()
    cfg = load_config(root / 'settings.json')
    assert all(s['host'].endswith('.invalid') and s['security'] == 'nts_required' for s in cfg['servers'])
    backend = WindowsServices()
    before = {name: backend.snapshot(name) for name in (*CONTROLLERS, SERVICE_NAME)}
    config_before = (root / 'settings.json').read_bytes()
    store = Store(root / 'data/history.sqlite3')
    _, decision = store.latest_decision()
    assert decision
    prepared = copy.deepcopy(decision)
    prepared.update(utc=time.time(), synchronization=dict(ready=True, sample_utc=time.time(), offset_ms=0,
        explanation='CI ownership fixture only; no usable synchronization samples'))
    store.save_cycle(store.next_cycle(), [], prepared, {}, prepared['utc'])
    exe = Path(sys.argv[1])
    try:
        # Override the release gate only in this guarded source test process.
        # The installed UI/helper retains its locked release gate.
        from takeover import activate
        with patch('takeover.TAKEOVER_RELEASE_ENABLED', True):
            activate(root, backend)
        assert read_journal(root)['phase'] == 'active'
        own = backend.snapshot(SERVICE_NAME)
        assert own['running'] and own['account'].casefold() == ('NT SERVICE\\' + SERVICE_NAME).casefold(), own
        assert backend.privilege(None)
        for name in CONTROLLERS:
            current = backend.snapshot(name)
            if current['exists']:
                assert not current['running'] and current['start_type'] == 4, current
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            with store.connect() as db:
                started = db.execute("SELECT COUNT(*) FROM events WHERE kind='clock_controller_started'").fetchone()[0]
                slewed = db.execute("SELECT payload FROM events WHERE kind='clock_correction'").fetchall()
            if started:
                break
            time.sleep(.2)
        assert started, 'Virtual service account did not initialize its clock backend'
        assert all(json.loads(p[0])['action'] != 'slew' for p in slewed), 'Fixture unexpectedly steered the clock'
    finally:
        subprocess.run([str(exe), 'rollback'], check=True)
    assert read_journal(root)['phase'] == 'rolled_back'
    after = {name: backend.snapshot(name) for name in before}
    assert before == after, (before, after)
    assert (root / 'settings.json').read_bytes() == config_before
    assert not backend.privilege(None)
    assert store.latest_decision()[1] is not None
    print('PASS: real service takeover, dedicated virtual-account clock privilege, no slew without samples, exact rollback of service states/config; history retained')


if __name__ == '__main__':
    main()
