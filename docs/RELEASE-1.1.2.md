# NetTime Next 1.1.2

A small desktop fix on top of 1.1.1.

## Desktop

- **"Refresh measurements" is now "Reload view".** The sidebar button only
  re-reads the measurements the service has already stored, which the window
  does on its own every few seconds. It never asked the service to measure
  again, so the old name suggested something it didn't do. Hovering over it
  now explains this. The service still checks each source once per check
  interval (64 seconds by default).

No changes to clock correction, sources or the service. Upgrading from 1.1.1
keeps the saved clock drift, settings, history and clock ownership.

## Validation

- 82 unit tests, fmt and clippy pass.
- Not yet done: the physical clock-response and install/upgrade tests in a
  disposable Windows VM, which are waiting on the dedicated runner.

The installer is unsigned. Windows may show an unknown-publisher warning. Verify
the SHA-256 against `SHA256SUMS.txt`.
