"""Regression check against the installed service using ordinary-user grants."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from app import gui
from monitor import load_config
from runtime_paths import machine_root
from scripts.restricted_identity import restricted_identity
from storage import Store


def main():
    root = machine_root()
    with restricted_identity():
        # Prove this check is not accidentally bypassing the service ACLs.
        for path in (root / "data" / "reader-must-not-write.tmp", root / "data" / "history.sqlite3"):
            try:
                with path.open("ab"):
                    pass
            except PermissionError:
                continue
            raise AssertionError(f"Reader unexpectedly has write access: {path}")
        reader = Store(root / "data" / "history.sqlite3", read_only=True)
        _, decision = reader.latest_decision()
        assert decision and decision["servers"], "No recorded service evidence"
        reader.stats()
        first = decision["servers"][0]["latest"]
        if first:
            reader.history(first["host"], first["port"])
        # Construct the real desktop service view under the same restricted token.
        gui(load_config(root / "settings.json"), reader, smoke=True,
            config_path=root / "settings.json", service_view=True)
    print("PASS: installed history and service-view desktop open with read-only user permissions")


if __name__ == "__main__":
    main()
