"""Stable application paths independent of the service's working directory."""
import os
import sys
from pathlib import Path

VERSION = "0.3.3"
SERVICE_NAME = "NetTimeNext"
SERVICE_DISPLAY_NAME = "NetTime Next Observer"
RESOURCE_DIR = Path(getattr(sys, "_MEIPASS", Path(__file__).resolve().parent))


def machine_root():
    return Path(os.environ.get("ProgramData", r"C:\ProgramData")) / "NetTimeNext"


def default_config():
    return RESOURCE_DIR / "settings.example.json"


def installed_service_exe():
    return Path(sys.executable).resolve().parent.parent / "Service" / "NetTimeNextService.exe"
