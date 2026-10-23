# NetTime Next 1.0.0

Network time monitoring and controlled synchronization for Windows 10/11 x64.
The desktop and automatic Windows service run natively without a Python runtime.

## Install

Download `NetTimeNext-1.0.0-Setup-x64.exe` from the GitHub release and run it
with administrator approval. Verify its SHA-256 against `SHA256SUMS.txt`.
This installer is unsigned; Windows may show an unknown-publisher warning.

The installer starts the background service with Windows and offers a system-tray
icon at sign-in. Minimise keeps the desktop on the taskbar. X hides it in the tray.
Click the tray icon to reopen it; Exit desktop app leaves the service running.

New installations start in measurement mode. Configure your sources, let their
filters settle, then choose Settings > Clock control > Take over clock when you
want NetTime Next to control Windows time. Clock takeover needs administrator
approval and at least three eligible independent source groups. Corrections are
gradual; large offsets suspend correction instead of stepping the clock.

Upgrading preserves configured sources, history, service identity, startup state
and active clock ownership. Existing installations retain their directory and
internal service/data names for compatibility. An upgrade never silently hands
the clock back to another service. Uninstall restores the saved ownership state
and preserves history and settings.

## Sources and security

Every configured endpoint is checked. Overview shows stratum, offset, round trip,
reach, authentication and selection reasons. Endpoints sharing an upstream clock
should share a source group; a group gets one timing vote. Separate group names
are an administrator assertion, not proof of physical independence.

New installations use NTS-required sources from three providers:
[Cloudflare](https://developers.cloudflare.com/time-services/ntp/usage/),
[Netnod](https://www.netnod.se/nts/network-time-security), and
[DFM](https://dfm.dk/ntp/). Change these in Settings for your location or network.
The defaults are not copied over an existing installation's settings.

NTS uses verified TLS 1.3 key exchange and authenticated NTP replies. An NTS-required
source never silently falls back to plain NTP. Certificates require a reasonably
correct initial system date. Reach and authentication establish successful
communication; they do not by themselves prove accurate server timestamps.

## History and settings

History logging is on by default, with seven-day retention. Settings > Collection
& history lets you switch logging off, enable detailed diagnostics, or choose
1-3650 days of retention. Live clock control and current status continue when
logging is off. Only the current status and minimal peer/safety state are kept.

Records older than 24 hours are compressed inside the database. Background cleanup
runs after startup/settings restart and hourly, removes records older than the
retention limit, and reclaims disk space. Reducing retention requires confirmation
before older measurements, decisions and events are permanently deleted. Export
first if you need to keep them. The ownership rollback journal is never purged.

Graphs and comparisons read recent and compressed records transparently. Source
comparison exports CSV/JSON summaries; full-history exports are `samples.json.gz`,
`decisions.json.gz` and `events.json.gz`. Exported files are managed by the user.
Normal logs avoid repeated full snapshots; detailed diagnostics use more space.

Settings changes preserve ownership and test changed sources before restarting.
Failed source checks leave the running collector untouched. Failed updates restore
previous settings. An interrupted update recovers on the next administrator action.
Filters settle again after a restart, usually around 8-10 minutes with 64-second polls.

## Supported scope and validation

This release is a Windows NTPv4 client with NTS support and slew-only clock control.
It is not an NTP server. Formal full-RFC conformance certification and extended
seven-day controlled-clock endurance validation have not been completed. There is
no claim of certified accuracy or independent security certification.

Release checks include protocol vectors, NTS authentication, source selection,
clock-direction response on a real Windows actuator, ownership rollback and
failure injection, installed upgrade/uninstall, compressed-history integrity,
retention boundaries, and taskbar/tray restore. Release assets include the exact
source/build manifest and Windows clock-response/tray reports.

## Build and administration

The application is built from `native/`. Build tooling uses Rust/MSVC, Python and
Inno Setup; these tools are not required to run the installed application.

```
cd native
python package.py --iscc C:\path\to\ISCC.exe
```

`NetTimeNextNativeService.exe --help` lists administrator and diagnostic commands.
`--health` reports current service ownership and measurements. Full-history export:
`--export FOLDER --database FILE`. Comparison export:
`--compare-export FOLDER --days 7 --database FILE`.
Manual cleanup: `--maintain-history --database FILE --days DAYS` (live installed
storage needs administrator rights). Cleanup permanently removes expired history.
The desktop accepts `--tray` to start directly in the notification area.
