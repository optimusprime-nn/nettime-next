# New network time utility: requirements and delivery status

## Operating alongside the existing version

The current NetTime service remains running and remains the clock controller.
The new utility begins as an observer. A future synchronization release requires
an explicit takeover flow that detects existing clock controllers and prevents
two independent correction loops from acting simultaneously.

## Every configured server is monitored

Query every configured source on its own schedule, including sources rejected
for synchronization. Keep measurement health separate from selection status.
Replace ambiguous “Not Used” with explicit states and reasons: waiting, valid,
timeout, network failure, invalid reply, authentication failure, stale,
disagrees, corroborating, selected, rate-limited, access refused or disabled by
the operator. Respect protocol access controls and rate limits; never promise
that “check all” bypasses a server's restrictions.

## Stratum and clock quality

Display and log stratum for each response, including changes over time. Preserve
stratum 0 control responses as control events and stratum 16 as unsynchronized,
not valid time sources. Show “unknown” when no response is available.
Also expose reference ID, leap/synchronization status, offset, delay, jitter,
root delay, root dispersion, freshness, reachability and authentication.
Stratum is a hierarchy level, not an accuracy score.

## Seven-day evidence by default

Retain at least seven days of attempts, replies, errors, source-quality metrics,
and source-selection decisions. Always capture enough detail to reconstruct
why each clock was selected or rejected; this is standard logging, not an
optional debug level. Log policy/configuration versions and thresholds used.
Record source switches, clock corrections (before/after, requested/applied),
restarts, suspend/resume gaps, connectivity changes and authentication outcomes.
Export measurements and decisions with timestamps and stable linkage.
The initial observer has measurements/decisions/events and gap/discontinuity
detection; explicit Windows power/network event subscription comes later.

## Settings redesign based on the supplied screenshots

| Page | Controls and information |
| --- | --- |
| Servers | Resizable list with add/edit/remove; hostname; NTP endpoint port; security policy; per-source health and test results. No fixed five-row ceiling. |
| Security & NTS | First-class NTS requirement per source; trust/certificate identity; NTS-KE endpoint and negotiated NTP endpoint; authentication and fallback status. |
| Polling | Normal interval with units; bounded retry/backoff; timeout; clearly described recovery. All entries continue to be monitored. |
| Clock correction | Observer versus synchronization mode; controller ownership; gradual correction; large-step limits/actions; holdover limits. Disabled while the old service owns the clock. |
| History & logs | Seven-day minimum; visible data location; coverage/gaps; exports; retention; always-on decision and security evidence. |
| Advanced / local server (later) | Service/startup behavior and optional serving of time, separated from client configuration. Serving is disabled by default; never advertise unsynchronized time as valid. |

Remove RFC868 TCP/UDP from the normal NTP setup. Expose legacy support separately
only if there is a concrete requirement. Replace “demote after four failures”
with an explainable health/recovery policy that does not stop collecting data.
Replace the broad “adjust system time” dropdown with explicit correction modes
and consequences. Keep software updates separate from time synchronization.

The working observer includes the first five settings pages. NTS-required is
explicitly blocked pending implementation; it is not an available NTS client.
Current settings save for restart. Hot application, per-source test buttons and
additional advanced/service controls are future work.

## NTP compliance gate

Target RFC 5905 with applicable updates/errata and RFC 8633 best practices.
Before enabling clock control, implement and validate the supported NTPv4
unicast client profile end to end: on-wire checks, association/reachability state,
clock filtering, selection, clustering, combining, poll behavior, loop/freshness
checks, leap handling, era handling, clock discipline and actual correction
verification. Define supported authentication and extension profiles explicitly.

Conformance needs a documented matrix and interoperability tests with independent
implementations, plus injected packet loss, reordering/replay, bogus timestamps,
source disagreement, KoD, clock jumps, sleep/resume, network loss and 2036
rollover. Passing unit tests for an observer is not a full compliance claim.

## NTS compliance gate

Implement RFC 8915, including NTS-KE with TLS certificate/hostname validation,
ALPN and record negotiation, TLS exporter key derivation, negotiated NTP endpoint,
AEAD authentication, unique identifiers/replay defenses, cookie lifecycle and
key renewal. Respect NTS-KE retry limits and TLS requirements. Default NTS-KE
port is TCP 4460; do not assume the negotiated NTP endpoint is identical to it.

“NTS required” must fail closed: a certificate, handshake or authenticated-packet
failure cannot silently become plain NTP. Any future optional fallback requires
an explicit policy and prominent downgrade event. Do not assume a configured
server offers NTS merely because it offers NTP or is stratum 1.

Log authentication status, certificate identity/failure category, negotiation
outcome, renewals and downgrade policy decisions. Do not log session keys,
reusable cookies or other secrets. Account for certificate validation when the
local date is incorrect without silently disabling validation. Authentication
does not prove clock accuracy; selection must still evaluate quality/diversity.

## Present build

Implemented: read-only all-source collector, stratum/security columns, redesigned
settings, seven-day rolling SQLite evidence, CSV exports, history graph,
explainable experimental recommendation, base-header validation, KoD controls,
restart-persistent scheduling and single collector lock.

Not implemented: NTS exchange/authentication, full NTP mitigation/discipline,
clock adjustment, installed Windows service, startup registration, local NTP
serving, signed standalone installer and interoperability certification.
