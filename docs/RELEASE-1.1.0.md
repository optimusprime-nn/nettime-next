# NetTime Next 1.1.0

This is the first public release since 1.0.0. It includes the unpublished 1.0.1
changes and a review of the native client.

## Clock correction

- **Large errors are corrected at once.** When the minimum number of
  independent source groups (three by default) agree that the PC clock is more
  than the 128 ms tracking limit out, NetTime Next sets the clock to the agreed
  time instead of recovering gradually over hours. Smaller errors are still
  trimmed gradually at a bounded rate. The clock is never set on unagreed,
  stale or pre-step evidence, or while clock ownership is unverified.
- **Restarts start on time.** Setting the clock also updates the PC's hardware
  clock, which Windows reads at boot. Before this, a PC under NetTime Next
  control could restart several seconds off.
- The 10-second automatic recovery limit setting is no longer used. Existing
  settings files still load.
- Frequency estimation uses the rates actually applied between samples and no
  longer jumps when the system peer switches between sources polled together.
- Correction pauses, instead of stopping, when clock ownership can't be checked.
- Hyper-V guest time synchronization is stopped while NetTime Next owns the
  clock, and restored on rollback.

## Sources and NTS

- One lost reply no longer removes a source from selection, and a single failed
  NTS exchange no longer discards the session.
- NTS key exchange is bounded by one overall deadline, so a slow or trickling
  server can't stall polling.
- A slow source no longer holds back every other source's results.
- Every resolved address is tried, not only the first.
- RATE and DENY kiss-o'-death replies are handled and recovered from.
- Stray datagrams are discarded instead of failing the poll.
- Each source group votes through its best source.
- 24 bits of the NTP origin timestamp are randomized.

## Desktop

- Opening NetTime Next again brings the running window forward instead of
  starting another copy.
- Starting in the tray at sign-in no longer shows the window.
- History and service state are read off the UI thread.

## Installation and service

- Upgrades from the Rust previews move to the NetTime Next Start menu folder.
- The data folder is secured before a fresh install uses it.
- Upgrades keep the service start type and restart the collector if an upgrade
  is cancelled or fails.
- Standard users can't hold the administration lock.
- History writes are deferred while another reader holds the database.

## Validation

- 77 unit tests, including new tests that forged and malformed NTS replies are
  rejected, and a standards conformance matrix in `docs/CONFORMANCE.md`.
- The step behaviour was exercised live on one workstation: the service set a
  7.49 s error in one step after sources agreed, and normal tracking resumed.
- The desktop's tray lifecycle checks pass. Single-instance behaviour was
  checked end to end.
- Not yet done: the physical clock-response and install/upgrade tests in a
  disposable Windows VM, which are waiting on the dedicated runner.

The installer is unsigned. Windows may show an unknown-publisher warning. Verify
the SHA-256 against `SHA256SUMS.txt`.
