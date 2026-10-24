# NetTime Next 1.0.0

First Windows x64 release of the native desktop and background time service.

Publication privacy rebuild: compiler paths are anonymised and checked during
packaging. Repository examples and historical commit identities have also been
sanitised. The replacement installer has a new checksum; use the attached
`SHA256SUMS.txt`. Earlier preview downloads have been withdrawn.

- Check every configured NTP/NTS endpoint and explain source selection.
- Show stratum, reach, offset, round trip and authentication alongside history.
- Administrator-controlled clock takeover, bounded gradual correction and durable rollback.
- Editable/reorderable sources, authenticated-source preflight and ownership-preserving upgrades.
- Seven-day logging by default, optional logging/diagnostics, configurable retention,
  24-hour compression, automatic purge and confirmation before shortening retention.
- Compact dark/light desktop; minimise stays on the taskbar, X hides in the tray.
- Automatic service at boot and optional native tray startup at sign-in.
- Public NTS defaults for new installations; existing settings and history are preserved.

The installer is unsigned. Formal full-RFC conformance certification and extended
seven-day controlled-clock endurance validation remain uncompleted. This release
is not an NTP server and does not step large clock errors. See README for supported
scope, configuration and recovery behaviour.
