"""Administrative clock ownership transaction with durable rollback evidence.

Only explicit takeover/rollback commands change other services. Merely installing
or opening the application never activates clock control.
"""
import json
from pathlib import Path
import time

from runtime_paths import SERVICE_NAME, machine_root

CONTROLLERS = ('NetTimeSvc', 'W32Time')
# Release gate, deliberately not a user-editable setting. See the conformance
# matrix: leap/step behaviour and the seven-day campaign remain unvalidated.
TAKEOVER_RELEASE_ENABLED = False


def write_json(path, value):
    path = Path(path)
    temporary = path.with_suffix('.tmp')
    temporary.write_text(json.dumps(value, indent=2) + '\n', encoding='utf-8')
    temporary.replace(path)


def read_journal(root):
    path = Path(root) / 'clock-ownership.json'
    return json.loads(path.read_text(encoding='utf-8')) if path.exists() else None


def readiness(cfg, decision, now=None):
    now = time.time() if now is None else now
    assessment = (decision or {}).get('synchronization', {})
    if not decision or not assessment.get('ready'):
        return False, assessment.get('explanation', 'No synchronization assessment yet')
    if not 0 <= now - decision.get('utc', 0) <= min(180, 2 * cfg['poll_seconds']):
        return False, 'Collector evidence is stale; wait for fresh measurements'
    if not 0 <= now - assessment.get('sample_utc', 0) <= cfg['clock']['holdover_seconds']:
        return False, 'Filtered synchronization sample is stale'
    if abs(assessment['offset_ms']) > cfg['clock']['max_offset_ms']:
        return False, 'Offset exceeds the slew limit; clock steps require manual review'
    return True, assessment['explanation']


class WindowsServices:
    def __init__(self):
        import win32service
        self.api = win32service

    def handle(self, name, access):
        service = self.api
        scm = service.OpenSCManager(None, None, service.SC_MANAGER_CONNECT)
        try:
            return service.OpenService(scm, name, access)
        finally:
            scm.Close()

    def snapshot(self, name):
        s = self.api
        try:
            handle = self.handle(name, s.SERVICE_QUERY_CONFIG | s.SERVICE_QUERY_STATUS)
        except Exception as exc:
            if getattr(exc, 'winerror', None) == 1060:
                return dict(name=name, exists=False)
            raise
        try:
            config = s.QueryServiceConfig(handle)
            state = s.QueryServiceStatus(handle)[1]
            if state not in (s.SERVICE_RUNNING, s.SERVICE_STOPPED):
                raise RuntimeError(f'{name} is changing state; wait before takeover')
            return dict(name=name, exists=True, start_type=config[1], account=config[7], running=state == s.SERVICE_RUNNING)
        finally:
            handle.Close()

    def configure(self, name, start_type=None, account=None):
        s = self.api
        handle = self.handle(name, s.SERVICE_CHANGE_CONFIG)
        try:
            s.ChangeServiceConfig(handle, s.SERVICE_NO_CHANGE,
                start_type if start_type is not None else s.SERVICE_NO_CHANGE,
                s.SERVICE_NO_CHANGE, None, None, 0, None, account, '' if account else None, None)
        finally:
            handle.Close()

    def running(self, name):
        s = self.api
        try:
            handle = self.handle(name, s.SERVICE_QUERY_STATUS)
        except Exception as exc:
            if getattr(exc, 'winerror', None) == 1060:
                return False
            raise
        try:
            return s.QueryServiceStatus(handle)[1] != s.SERVICE_STOPPED
        finally:
            handle.Close()

    def change(self, name, running):
        from service_control import wait_status
        s = self.api
        handle = self.handle(name, s.SERVICE_QUERY_STATUS | s.SERVICE_START | s.SERVICE_STOP)
        try:
            state = s.QueryServiceStatus(handle)[1]
            if running and state != s.SERVICE_RUNNING:
                s.StartService(handle, None)
                wait_status(handle, s.SERVICE_RUNNING)
            elif not running and state != s.SERVICE_STOPPED:
                if state != s.SERVICE_STOP_PENDING:
                    s.ControlService(handle, s.SERVICE_CONTROL_STOP)
                wait_status(handle, s.SERVICE_STOPPED)
        finally:
            handle.Close()

    def privilege(self, grant):
        import win32security as security
        sid = security.LookupAccountName(None, 'NT SERVICE\\' + SERVICE_NAME)[0]
        policy = security.LsaOpenPolicy(None, security.POLICY_ALL_ACCESS)
        try:
            try:
                existing = security.LsaEnumerateAccountRights(policy, sid)
            except Exception as exc:
                if getattr(exc, 'winerror', None) not in (2, 259):
                    raise
                existing = []
            had = 'SeSystemtimePrivilege' in existing
            if grant and not had:
                security.LsaAddAccountRights(policy, sid, ['SeSystemtimePrivilege'])
            elif not grant and had:
                security.LsaRemoveAccountRights(policy, sid, False, ['SeSystemtimePrivilege'])
            return had
        finally:
            policy.Close()


def ownership_guard(root=None, backend=None):
    root = root or machine_root()
    journal = read_journal(root)
    if not journal or journal.get('phase') != 'active':
        return False
    backend = backend or WindowsServices()
    return not any(backend.running(name) for name in CONTROLLERS)


def activate(root=None, backend=None):
    from monitor import load_config
    from service_control import require_admin
    from storage import Store
    require_admin()
    if not TAKEOVER_RELEASE_ENABLED:
        raise RuntimeError('Clock takeover is locked in this preview until NTP conformance and seven-day reliability validation are complete')
    root = Path(root or machine_root())
    backend = backend or WindowsServices()
    previous = read_journal(root)
    if previous and previous.get('phase') != 'rolled_back':
        raise RuntimeError('A clock ownership record already exists; restore the previous controller first')
    cfg = load_config(root / 'settings.json')
    store = Store(root / 'data/history.sqlite3', cfg['retention_days'])
    _, decision = store.latest_decision()
    ready, reason = readiness(cfg, decision)
    if not ready:
        raise RuntimeError('Takeover is not ready: ' + reason)
    own = backend.snapshot(SERVICE_NAME)
    if not own['exists'] or not own['running']:
        raise RuntimeError('NetTime Next service must be running before takeover')
    if own['account'].casefold() not in ('nt authority\\localservice', 'nt service\\' + SERVICE_NAME.casefold()):
        raise RuntimeError('Takeover requires the standard service account; custom accounts are not changed')
    journal = dict(version=1, phase='prepared', utc=time.time(), own_service=own,
                   controllers=[backend.snapshot(name) for name in CONTROLLERS],
                   previous_config=json.loads((root / 'settings.json').read_text(encoding='utf-8-sig')),
                   previous_config_text=(root / 'settings.json').read_bytes().decode('utf-8'),
                   privilege_preexisting=backend.privilege(None))
    path = root / 'clock-ownership.json'
    write_json(path, journal)  # Durable intent *before* changing any service.
    store.event('takeover_prepared', {'services': journal['controllers'], 'assessment': decision['synchronization']})
    try:
        backend.change(SERVICE_NAME, False)
        backend.privilege(True)
        for service in journal['controllers']:
            if service['exists']:
                backend.change(service['name'], False)
                backend.configure(service['name'], start_type=4)  # Disabled until explicit rollback.
        # A dedicated virtual account confines the added privilege to this service.
        backend.configure(SERVICE_NAME, account='NT SERVICE\\' + SERVICE_NAME)
        cfg['clock']['mode'] = 'slew'
        write_json(root / 'settings.json', cfg)
        journal['phase'] = 'active'
        write_json(path, journal)
        backend.change(SERVICE_NAME, True)
        store.event('takeover_active', {'mode': 'slew', 'limits': cfg['clock']})
    except Exception as exc:
        store.event('takeover_failed', {'reason': str(exc), 'action': 'Restoring previous ownership'})
        rollback(root, backend)
        raise


def rollback(root=None, backend=None):
    from service_control import require_admin
    from storage import Store
    require_admin()
    root = Path(root or machine_root())
    backend = backend or WindowsServices()
    journal = read_journal(root)
    if not journal or journal['phase'] == 'rolled_back':
        return
    # Mark inactive first so a service restart cannot regain clock ownership.
    journal['phase'] = 'rolling_back'
    write_json(root / 'clock-ownership.json', journal)
    backend.change(SERVICE_NAME, False)
    # Stop also covers interrupted activation before the controller was created.
    from clock_control import WindowsClock
    if isinstance(backend, WindowsServices):
        WindowsClock().restore()
    if 'previous_config_text' in journal:
        config = root / 'settings.json'
        temporary = config.with_suffix('.tmp')
        temporary.write_bytes(journal['previous_config_text'].encode('utf-8'))
        temporary.replace(config)
    else:
        write_json(root / 'settings.json', journal['previous_config'])
    backend.configure(SERVICE_NAME, account=journal['own_service']['account'])
    if journal['privilege_preexisting'] is False:
        backend.privilege(False)
    errors = []
    for service in journal['controllers']:
        if service['exists']:
            try:
                backend.configure(service['name'], start_type=service['start_type'])
                backend.change(service['name'], service['running'])
            except Exception as exc:
                errors.append(f"{service['name']}: {exc}")
    if errors:
        raise RuntimeError('Rollback incomplete; retry Restore previous controller: ' + '; '.join(errors))
    backend.change(SERVICE_NAME, journal['own_service']['running'])
    journal['phase'] = 'rolled_back'
    journal['rollback_utc'] = time.time()
    write_json(root / 'clock-ownership.json', journal)
    Store(root / 'data/history.sqlite3').event('takeover_rolled_back', {'restored': journal['controllers']})
