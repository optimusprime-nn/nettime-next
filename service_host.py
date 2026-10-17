"""Frozen Windows SCM entry point and administrative helper.

The service uses LocalService and a service-specific writable data directory.
It never starts a desktop UI or changes the system time.
"""
import argparse
import logging
import logging.handlers
import sys
import threading

import servicemanager
import win32service
import win32serviceutil
import win32timezone  # Required by the frozen pywin32 runtime.

from monitor import InstanceLock, Monitor, load_config
from runtime_paths import SERVICE_NAME, SERVICE_DISPLAY_NAME, VERSION, machine_root
from service_control import action, apply_config, install, query_status
from storage import Store


def logger():
    log = logging.getLogger("NetTimeNext")
    if not log.handlers:
        path = machine_root() / "data" / "service.log"
        path.parent.mkdir(parents=True, exist_ok=True)
        handler = logging.handlers.TimedRotatingFileHandler(path, when="midnight", utc=True,
                                                           backupCount=7, encoding="utf-8")
        handler.setFormatter(logging.Formatter("%(asctime)s %(levelname)s %(message)s"))
        log.addHandler(handler)
        log.setLevel(logging.INFO)
    return log


class ServiceMessages:
    def put(self, item):
        if item[0] == "error":
            logger().error("%s", item[1])


class ObserverService(win32serviceutil.ServiceFramework):
    _svc_name_ = SERVICE_NAME
    _svc_display_name_ = SERVICE_DISPLAY_NAME
    _svc_description_ = "Read-only all-source network time monitoring"

    def __init__(self, args):
        super().__init__(args)
        self.shutdown = threading.Event()
        self.monitor = None

    def SvcStop(self):
        self.ReportServiceStatus(win32service.SERVICE_STOP_PENDING, waitHint=30000)
        self.shutdown.set()
        if self.monitor:
            self.monitor.stop.set()

    def SvcShutdown(self):
        self.SvcStop()

    def SvcRun(self):
        self.ReportServiceStatus(win32service.SERVICE_START_PENDING, waitHint=30000)
        self.SvcDoRun()
        self.ReportServiceStatus(win32service.SERVICE_STOPPED)

    def SvcDoRun(self):
        root = machine_root()
        try:
            logger().info("Observer service %s starting; measurement only", VERSION)
            cfg = load_config(root / "settings.json")
            store = Store(root / "data" / "history.sqlite3", cfg["retention_days"])
            with InstanceLock(root / "data" / "monitor.lock"):
                self.monitor = Monitor(cfg, store, ServiceMessages())
                if self.shutdown.is_set():
                    self.monitor.stop.set()
                self.ReportServiceStatus(win32service.SERVICE_RUNNING)
                errors = []

                def run():
                    try:
                        self.monitor.run(raise_errors=True)
                    except Exception as exc:
                        errors.append(exc)

                thread = threading.Thread(target=run, name="NTP-collector")
                thread.start()
                while thread.is_alive():
                    thread.join(1)
                    if self.shutdown.is_set():
                        self.ReportServiceStatus(win32service.SERVICE_STOP_PENDING, waitHint=30000)
                if errors:
                    raise RuntimeError(str(errors[0]))
            logger().info("Observer service stopped cleanly")
        except Exception:
            logger().exception("Observer service failed")
            self.ReportServiceStatus(win32service.SERVICE_STOPPED, win32ExitCode=1066, svcExitCode=1)
            raise


def main():
    if len(sys.argv) == 1:
        servicemanager.Initialize()
        servicemanager.PrepareToHostSingle(ObserverService)
        servicemanager.StartServiceCtrlDispatcher()
        return
    parser = argparse.ArgumentParser(description="NetTime Next Windows service")
    parser.add_argument("command", choices=("install", "start", "stop", "restart", "remove", "status", "health", "apply-config", "version"))
    parser.add_argument("--file")
    args = parser.parse_args()
    if args.command == "version":
        print(VERSION)
    elif args.command == "status":
        print(query_status())
    elif args.command == "health":
        import json
        root = machine_root()
        _, decision = Store(root / "data" / "history.sqlite3", read_only=True).latest_decision()
        print(json.dumps({"service": query_status(), "version": VERSION,
                          "latest_decision_utc": decision["utc"] if decision else None,
                          "servers": len(decision["servers"]) if decision else 0,
                          "mode": "measurement only"}))
    elif args.command == "install":
        install()
    elif args.command == "apply-config":
        if not args.file:
            parser.error("apply-config requires --file")
        apply_config(args.file)
    else:
        action(args.command)


if __name__ == "__main__":
    try:
        main()
    except Exception:
        try:
            logger().exception("Service command failed")
        except Exception:
            pass
        raise
