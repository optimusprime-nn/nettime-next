# Legacy Python implementation

This folder holds the original Python implementation of NetTime Next (0.2–0.3
series). It is kept as a compatibility and parity reference. The supported
product is the native Rust application in [`../native`](../native).

| Path | Contents |
| --- | --- |
| `*.py` | Desktop app, Windows service host, NTP/NTS client, selection and clock control |
| `tests/` | Unit and contract tests |
| `scripts/` | Installed-build, takeover and clock-response checks |
| `installer/` | Inno Setup script for the Python build |
| `build.py` | Builds the executables and installer into `legacy/dist` |

Run the tests from this folder:

```text
python -m unittest discover -s tests -t . -p "test_*.py"
```

The shared icon and Windows manifest live in [`../assets`](../assets).
