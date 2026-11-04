---
layout: default
title: "Scope and validation"
---
# Scope and validation

## What NetTime Next is

- An NTPv4 **client** for Windows 10/11 x64, with NTS support.
- **Agreement-gated** clock control. Small errors are slewed by changing
  the clock's rate; errors above the tracking limit are set at once when the
  sources agree.
- A measurement and evidence tool. Every decision is recorded with its
  reasons.

## What it isn't

- Not an NTP **server**. It doesn't answer time requests from other
  machines.
- Not certified. There is no claim of certified accuracy, formal RFC
  conformance, or independent security certification.
- Not a replacement for domain time in an Active Directory domain unless
  whoever manages the domain agrees.

## What has been tested

Release checks cover:

- NTP protocol test vectors, and NTS key exchange and packet authentication,
  including tampered, replayed, wrong-key and malformed packets,
- source filtering, selection and clustering,
- clock correction in both directions against simulated clocks, including
  outages, setting the clock on agreed large offsets, and holdover,
- clock direction measured against a real Windows clock actuator,
- clock ownership takeover, rollback and failure injection,
- installed upgrade and uninstall,
- compressed-history integrity and retention boundaries,
- taskbar and tray behaviour.

Each release's assets include a build manifest that names the exact source
commit.

## Not yet done

- Formal RFC 5905 conformance review.
- Seven-day controlled-clock endurance runs.
- A clean-machine test matrix across Windows 10 and 11 editions.
- Code signing for the installer.
- An independent security review of the NTS implementation.

## Standards

- [RFC 5905](https://www.rfc-editor.org/rfc/rfc5905.html): NTPv4 protocol
  and algorithms ([errata](https://www.rfc-editor.org/errata_search.php?rfc=5905)).
- [RFC 8915](https://www.rfc-editor.org/rfc/rfc8915.html): Network Time
  Security.
- [RFC 8633](https://www.rfc-editor.org/rfc/rfc8633.html): NTP best current
  practices.
- [SetSystemTimeAdjustmentPrecise](https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-setsystemtimeadjustmentprecise):
  the Windows API used to adjust the clock rate.

## Third-party notices

Third-party components and their licences are listed in
`THIRD-PARTY-NOTICES.md` in the repository. The installer also places the
licence texts in the `ThirdPartyLicenses` folder in the program folder.
