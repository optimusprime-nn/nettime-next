"""Build two self-contained Windows executables and an Inno Setup installer."""
import argparse
import hashlib
import json
import os
import subprocess
import sys
import shutil
import importlib.metadata
from pathlib import Path

from runtime_paths import VERSION

ROOT = Path(__file__).resolve().parent


def run(*command):
    subprocess.run(list(map(str, command)), cwd=ROOT, check=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--iscc", type=Path, required=True)
    parser.add_argument("--skip-tests", action="store_true")
    args = parser.parse_args()
    if os.name != "nt":
        raise SystemExit("Build Windows executables on Windows")
    if not args.skip_tests:
        run(sys.executable, "-m", "unittest", "discover", "-s", ".", "-p", "test_*.py", "-v")
    common = [sys.executable, "-m", "PyInstaller", "--noconfirm", "--clean", "--noupx", "--onedir",
              "--add-data", "settings.example.json;."]
    run(*common, "--name", "NetTimeNext", "--windowed", "app.py")
    run(*common, "--name", "NetTimeNextService", "--console", "service_host.py")
    notices = ROOT / "dist" / "THIRD-PARTY-LICENSES"
    notices.mkdir(exist_ok=True)
    for package in ("pywin32", "pyinstaller", "altgraph", "pywin32-ctypes", "pefile", "packaging"):
        distribution = importlib.metadata.distribution(package)
        for file in distribution.files or []:
            if "license" in str(file).lower() or Path(file).name.lower() == "copying.txt":
                source = Path(distribution.locate_file(file))
                if source.is_file():
                    shutil.copy2(source, notices / (package + "-" + source.name))
    for relative in ("LICENSE.txt", "tcl/tcl8.6/license.terms", "tcl/tk8.6/license.terms"):
        source = Path(sys.base_prefix) / relative
        if source.is_file():
            shutil.copy2(source, notices / relative.replace("/", "-"))
    run(args.iscc.resolve(), f"/DAppVersion={VERSION}", ROOT / "installer" / "NetTimeNext.iss")
    setup = ROOT / "dist" / f"NetTimeNext-{VERSION}-Setup-x64.exe"
    manifest = dict(version=VERSION, installer=setup.name,
                    sha256=hashlib.sha256(setup.read_bytes()).hexdigest(),
                    source_commit=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                    source_dirty=bool(subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=ROOT, text=True)),
                    mode="measurement-only Windows service", signed=False)
    (ROOT / "dist" / "build-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    (ROOT / "dist" / "SHA256SUMS.txt").write_text(f"{manifest['sha256']}  {setup.name}\n", encoding="utf-8")
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
