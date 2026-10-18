# Synchronization preview: implementation and validation gates

Version 0.3.0 progresses the four agreed work streams. Clock control is
implemented behind `takeover.TAKEOVER_RELEASE_ENABLED = False`. The shipped
UI/helper and ordinary settings saves cannot activate takeover.

| Requirement | Implementation/evidence | Outstanding gate |
|---|---|---|
| Source comparison | `comparison.py`, snapshot summaries, 24h/7d/retained view and exports; `test_comparison.py` | Actual seven-day collection, reboot/network/suspend campaign |
| NTS-KE | TLS ≥1.3, ALPN ntske/1, CA chain and DNS/IP identity, bounded negotiation, negotiated endpoint, directional TLS exporter | More public-server and Windows-network configurations |
| NTS packets | AES-SIV, random 256-bit identifier, authenticated header, encrypted replenishment cookies; tamper/replay/wrong-key/malformed/unencrypted-cookie tests | Independent security review and broader malformed-packet corpus |
| NTS recovery | No plain fallback; consumed cookies, renewal, daily KE and persisted exponential retry | Crash/power-loss campaign; secrets intentionally not persisted |
| NTP filtering | Eight stages, missing stages MAXDISP, aged dispersion, minimum-RTT sample, RMS jitter and distance components | Trace comparison; full spike/state handling |
| NTP mitigation | Midpoint-aware correctness interval, clustering, stratum/distance ordering, weighted combination, full reasons | RFC errata review, broader interop/falseticker campaigns |
| Source diversity | Endpoint and operator-assigned group deduplication, three groups minimum, no unassigned votes | Operator verification of hardware/upstream independence |
| Discipline | Bounded slew planner, oscillator estimate, warmup, no sample reuse; simulated fast/slow clocks converge | Long-running Windows closed-loop validation; full RFC PLL/FLL state-machine profile |
| Large offsets/leaps | Suspend explicitly; never step or ignore leap announcements | Supported leap/step/stepout policy and validation |
| Ownership/rollback | Durable journal, virtual service identity and clock right, NetTimeSvc/W32Time exclusivity, conflict watchdog, partial-failure recovery | Other controllers, domain/group-policy, forced-kill and reboot recovery |
| Installation | Upgrade/read-only history checks, packaged TLS/CA check, disposable Windows ownership/privilege fixture | Windows 10/11 clean-machine matrix; signing |

Do not change the takeover release gate until the outstanding NTP profile and
endurance checks are reviewed and their evidence committed. Unit tests and NTS
interoperability alone do not establish complete RFC 5905 conformance. The legacy
diagnostic recommendation remains separate from the synchronization assessment.

No clock adjustment/ownership change is performed on the user's PC to validate
this preview. The disposable CI fixture requires GitHub-hosted runner markers
and reserved `.invalid`, NTS-only fixture sources. It overrides the release gate
only in the guarded source test process to exercise the actual Windows account,
clock privilege and neutral backend startup, then restores service states and
the exact configuration bytes. The frozen helper keeps its release gate locked.
The fixture supplies no usable time sample and permits no slew.

## Standards and interoperability

- [RFC 5905](https://www.rfc-editor.org/rfc/rfc5905.html), especially §§10–12.
- [RFC 5905 errata](https://www.rfc-editor.org/errata_search.php?rfc=5905).
- [RFC 8915](https://www.rfc-editor.org/rfc/rfc8915.html), NTS TLS/KE and extensions.
- [RFC 8633](https://www.rfc-editor.org/rfc/rfc8633.html), operational practices.
- [Windows precise rate API](https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-setsystemtimeadjustmentprecise).
- [Cloudflare NTS](https://developers.cloudflare.com/time-services/nts/).
- [Netnod NTS](https://www.netnod.se/nts/network-time-security).

Read-only live exchanges with `time.cloudflare.com` and `nts.netnod.se` each
passed TLS 1.3, certificate identity, NTS authentication and NTP timestamp checks.
A second exchange after 65 seconds reused each session successfully and restored
the cookie pool to eight. Both initial key establishment and authenticated cookie
renewal were exercised against independent implementations. These checks do not
prove all failure modes or long-term accuracy.

The first real Windows ownership test caught an invalid virtual-account password
argument and a precise-clock API lookup in kernel32 instead of the API set/KernelBase.
Both are covered by regression checks. Partial activation now avoids touching the
clock backend before that backend could have run, and silent uninstall returns
failure without a blocking dialog if recovery cannot complete.

Unit tests use local TLS/UDP fixtures and fake clocks; drift simulations never
change host time. The precise API export check resolves functions without calling
any adjustment API.
