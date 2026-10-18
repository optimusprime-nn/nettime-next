"""Small administrative surface for this application's own Windows service."""
from __future__ import annotations

import ctypes
import json
import subprocess
import sys
import time
from pathlib import Path

from monitor import load_config, validate_config
from runtime_paths import SERVICE_NAME, SERVICE_DISPLAY_NAME, default_config, machine_root
from storage import Store


def modules():
    import win32service
    import win32security
    return win32service, win32security


def require_admin():
    if not ctypes.windll.shell32.IsUserAnAdmin():
        raise PermissionError("Changing the observer service requires administrator approval")


def query_status():
    service, _ = modules()
    scm = service.OpenSCManager(None, None, service.SC_MANAGER_CONNECT)
    handle = None
    try:
        try:
            handle = service.OpenService(scm, SERVICE_NAME, service.SERVICE_QUERY_STATUS)
        except Exception as exc:
            if getattr(exc, "winerror", None) == 1060:
                return "not installed"
            raise
        state = service.QueryServiceStatus(handle)[1]
        return {1: "stopped", 2: "starting", 3: "stopping", 4: "running", 7: "paused"}.get(state, f"state {state}")
    finally:
        if handle:
            handle.Close()
        scm.Close()


def wait_status(handle, target, timeout=45):
    service, _ = modules()
    deadline = time.perf_counter() + timeout
    while time.perf_counter() < deadline:
        state = service.QueryServiceStatus(handle)
        if state[1] == target:
            return
        if target == service.SERVICE_RUNNING and state[1] == service.SERVICE_STOPPED:
            raise RuntimeError(f"Observer stopped during startup (Windows exit {state[3]}, service exit {state[4]})")
        time.sleep(.2)
    raise TimeoutError("Timed out waiting for the observer service")


def action(command):
    require_admin()
    service, _ = modules()
    scm = service.OpenSCManager(None, None, service.SC_MANAGER_CONNECT)
    handle = None
    try:
        try:
            handle = service.OpenService(scm, SERVICE_NAME,
                service.SERVICE_QUERY_STATUS | service.SERVICE_START | service.SERVICE_STOP | 0x00010000)
        except Exception as exc:
            if getattr(exc, "winerror", None) == 1060 and command in ("stop", "remove"):
                return
            raise
        state = service.QueryServiceStatus(handle)[1]
        if command in ("stop", "restart", "remove") and state != service.SERVICE_STOPPED:
            if state != service.SERVICE_STOP_PENDING:
                service.ControlService(handle, service.SERVICE_CONTROL_STOP)
            wait_status(handle, service.SERVICE_STOPPED)
        if command in ("start", "restart") and service.QueryServiceStatus(handle)[1] != service.SERVICE_RUNNING:
            service.StartService(handle, None)
            wait_status(handle, service.SERVICE_RUNNING)
        if command == "remove":
            service.DeleteService(handle)
    finally:
        if handle:
            handle.Close()
        scm.Close()


def set_permissions():
    service, security = modules()
    root = machine_root()
    sid, _, _ = security.LookupAccountName(None, "NT SERVICE\\" + SERVICE_NAME)
    service_sid = security.ConvertSidToStringSid(sid)
    for path, service_rights in ((root, "GRGX"), (root / "data", "0x1301bf")):
        # Administrators/System own configuration. Users can read evidence only.
        descriptor = security.ConvertStringSecurityDescriptorToSecurityDescriptor(
            "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;BU)"
            f"(A;OICI;{service_rights};;;{service_sid})", 1)
        security.SetFileSecurity(str(path), security.DACL_SECURITY_INFORMATION |
            security.PROTECTED_DACL_SECURITY_INFORMATION, descriptor)


def install(executable=None):
    require_admin()
    service, _ = modules()
    exe = Path(executable or sys.executable).resolve()
    if not exe.is_file():
        raise FileNotFoundError(exe)
    if executable is None and not getattr(sys, "frozen", False):
        raise RuntimeError("Install the built NetTimeNextService.exe, not the development Python interpreter")
    root = machine_root()
    (root / "data").mkdir(parents=True, exist_ok=True)
    config = root / "settings.json"
    if not config.exists():
        value = load_config(default_config())
        config.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    cfg = load_config(config)
    scm = service.OpenSCManager(None, None, service.SC_MANAGER_CREATE_SERVICE | service.SC_MANAGER_CONNECT)
    handle = None
    try:
        binary_path = f'"{exe}"'
        try:
            handle = service.OpenService(scm, SERVICE_NAME, service.SERVICE_ALL_ACCESS)
        except Exception as exc:
            if getattr(exc, "winerror", None) != 1060:
                raise
            handle = service.CreateService(scm, SERVICE_NAME, SERVICE_DISPLAY_NAME,
                service.SERVICE_ALL_ACCESS, service.SERVICE_WIN32_OWN_PROCESS,
                service.SERVICE_AUTO_START, service.SERVICE_ERROR_NORMAL,
                binary_path, None, 0, None, r"NT AUTHORITY\LocalService", None)
        else:
            previous = service.QueryServiceConfig(handle)[3]
            if previous.casefold() != binary_path.casefold():
                raise RuntimeError("A different executable already owns the NetTimeNext service; refusing to replace it")
            if service.QueryServiceStatus(handle)[1] != service.SERVICE_STOPPED:
                raise RuntimeError("Stop the observer service before installing an upgrade")
        service.ChangeServiceConfig2(handle, service.SERVICE_CONFIG_DESCRIPTION,
            "Measures every configured NTP source and retains seven days of evidence. Clock takeover requires explicit approval and release validation.")
        service.ChangeServiceConfig2(handle, service.SERVICE_CONFIG_SERVICE_SID_INFO, service.SERVICE_SID_TYPE_UNRESTRICTED)
        service.ChangeServiceConfig2(handle, service.SERVICE_CONFIG_DELAYED_AUTO_START_INFO, True)
        service.ChangeServiceConfig2(handle, service.SERVICE_CONFIG_FAILURE_ACTIONS, {
            "ResetPeriod": 86400, "RebootMsg": None, "Command": None,
            "Actions": [(service.SC_ACTION_RESTART, 5000), (service.SC_ACTION_RESTART, 30000),
                        (service.SC_ACTION_RESTART, 60000)]})
        service.ChangeServiceConfig2(handle, service.SERVICE_CONFIG_FAILURE_ACTIONS_FLAG, True)
        set_permissions()
        store = Store(root / "data" / "history.sqlite3", cfg["retention_days"])
        store.event("service_registered", {"executable": str(exe), "account": "LocalService",
            "mode": cfg["clock"]["mode"], "startup": "automatic (delayed)"})
    finally:
        if handle:
            handle.Close()
        scm.Close()


def apply_config(path):
    require_admin()
    source = Path(path)
    if source.stat().st_size > 131072:
        raise ValueError("Settings file exceeds size limit")
    cfg = validate_config(json.loads(source.read_text(encoding="utf-8-sig")))
    root = machine_root()
    config = root / "settings.json"
    previous = config.read_text(encoding="utf-8")
    old = json.loads(previous.lstrip("\ufeff"))
    if cfg['clock']['mode'] != old.get('clock', {}).get('mode', 'observe'):
        raise ValueError('Clock ownership can only change through Take over or Restore previous controller')
    running = query_status() == "running"
    action("stop")
    temp = config.with_suffix(".json.tmp")
    try:
        temp.write_text(json.dumps(cfg, indent=2) + "\n", encoding="utf-8")
        temp.replace(config)
        Store(root / "data" / "history.sqlite3", cfg["retention_days"]).event("settings_applied", {"old": old, "new": cfg})
        if running:
            action("start")
    except Exception as exc:
        temp.write_text(previous, encoding="utf-8")
        temp.replace(config)
        Store(root / "data" / "history.sqlite3").event("settings_rolled_back", {
            "restored": old, "attempted": cfg, "reason": str(exc)})
        if running:
            action("start")
        raise
