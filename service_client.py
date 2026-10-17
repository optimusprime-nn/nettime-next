"""Read service status normally; request elevation only for administrative changes."""
import ctypes
import ctypes.wintypes as wt
import json
import subprocess
import tempfile
from pathlib import Path

from runtime_paths import installed_service_exe


def elevated_command(command, *args):
    exe = installed_service_exe()
    if not exe.is_file():
        raise FileNotFoundError("Installed service helper was not found. Run the installer first.")

    class SHELLEXECUTEINFO(ctypes.Structure):
        _fields_ = [("cbSize", wt.DWORD), ("fMask", wt.ULONG), ("hwnd", wt.HWND),
            ("lpVerb", wt.LPCWSTR), ("lpFile", wt.LPCWSTR), ("lpParameters", wt.LPCWSTR),
            ("lpDirectory", wt.LPCWSTR), ("nShow", ctypes.c_int), ("hInstApp", wt.HINSTANCE),
            ("lpIDList", wt.LPVOID), ("lpClass", wt.LPCWSTR), ("hkeyClass", wt.HKEY),
            ("dwHotKey", wt.DWORD), ("hIcon", wt.HANDLE), ("hProcess", wt.HANDLE)]

    info = SHELLEXECUTEINFO()
    info.cbSize, info.fMask, info.lpVerb = ctypes.sizeof(info), 0x40 | 0x400, "runas"
    info.lpFile = str(exe)
    info.lpParameters = subprocess.list2cmdline([command, *map(str, args)])
    info.nShow = 0
    shell = ctypes.WinDLL("shell32", use_last_error=True)
    shell.ShellExecuteExW.argtypes = [ctypes.POINTER(SHELLEXECUTEINFO)]
    shell.ShellExecuteExW.restype = wt.BOOL
    if not shell.ShellExecuteExW(ctypes.byref(info)):
        raise OSError(ctypes.get_last_error(), "Administrator approval was cancelled or the service helper could not start")
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.WaitForSingleObject.argtypes = [wt.HANDLE, wt.DWORD]
    kernel.GetExitCodeProcess.argtypes = [wt.HANDLE, ctypes.POINTER(wt.DWORD)]
    kernel.CloseHandle.argtypes = [wt.HANDLE]
    try:
        kernel.WaitForSingleObject(info.hProcess, 0xFFFFFFFF)
        code = wt.DWORD()
        if not kernel.GetExitCodeProcess(info.hProcess, ctypes.byref(code)):
            raise ctypes.WinError(ctypes.get_last_error())
        if code.value:
            raise RuntimeError("Service change failed. See the service.log file in the data folder.")
    finally:
        kernel.CloseHandle(info.hProcess)


def save_service_config(draft):
    with tempfile.TemporaryDirectory(prefix="nettime-settings-") as folder:
        source = Path(folder) / "settings.json"
        source.write_text(json.dumps(draft, indent=2), encoding="utf-8")
        elevated_command("apply-config", "--file", source)
