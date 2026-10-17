# Audit of the TS1 / Raspberry Pi recommendation

## Identified sources

The operator identifies 192.0.2.54 as TS1, an ESP32-P4 GNSS-disciplined
stratum-1 source, and 192.0.2.62 as a GPS-disciplined Raspberry Pi 3B
stratum-1 source. GPS and PPS are their advertised reference identifiers.
Neither identifier classifies all attached hardware or proves UTC accuracy.
The software does not use reference ID to rank sources.

## Exact matching evidence

Stored decision 7 matches the pasted latest offsets, delays and jitter values.
The old table displayed the latest offset/delay but the recommendation used a
minimum-RTT sample from its recent history. Those were different TS1 samples:

| Quantity | TS1 | Raspberry Pi 3B |
| --- | ---: | ---: |
| Latest displayed offset, server minus PC | -10.944605 ms | -10.928154 ms |
| Latest displayed RTT | 0.171185 ms | 0.712872 ms |
| Actual ranking sample offset | -7.159114 ms | -10.928154 ms |
| Actual ranking sample RTT | 0.141382 ms | 0.712872 ms |
| Ranking sample age | 129.135075 s | 0.639395 s |

The experimental score is:

`RTT/2 + max(upstream root delay,0)/2 + root dispersion + age × 0.015 ms/s + max(1 ms, advertised precision) + RMS client-relative offset variation`

The 0.015 ms/s age allowance corresponds to 15 ppm. The fixed 1 ms observer
guard is a conservative policy allowance, not the server's hardware precision.
This is a diagnostic estimate, not the full RFC 5905 filter/selection pipeline
or an independently measured bound on absolute UTC error.

| Score component | TS1 | Raspberry Pi 3B |
| --- | ---: | ---: |
| Ranking RTT / 2 | 0.070691 ms | 0.356436 ms |
| Upstream root delay / 2 | 0 ms | 0 ms |
| Advertised root dispersion | 0.091553 ms | 1.678467 ms |
| Age allowance | 1.937026 ms | 0.009591 ms |
| Observer precision guard | 1.000000 ms | 1.000000 ms |
| RMS client-relative offset variation | 2.977158 ms | 2.941803 ms |
| **Total** | **6.076428 ms** | **5.986296 ms** |

The Pi won this specific decision by about 0.090132 ms. The arithmetic follows
the existing policy: it was not simply preferring a 35 microsecond jitter
advantage over a 542 microsecond latest-RTT advantage. The mixed sample epochs
and hidden age/dispersion contributions made the old display misleading.

## What the numbers do and do not measure

NetTime Next measures network time relative to the Windows PC, whose clock is
still controlled by another utility. PPS discipline at the servers does not
directly discipline this PC. Its roughly -11 ms displayed offsets mean the
servers were measured behind this PC; they do not establish an 11 ms GNSS/PPS
error. RMS offsets across polls include client drift/corrections, network
variation and source behavior. They do not measure physical PPS jitter.

The similar offsets in a single display support agreement at that client, but
do not prove microsecond UTC accuracy. Sources can have different measurement
epochs, shared upstreams, path asymmetry and userspace timestamp error. The
monitor has no direct PPS capture input or hardware receive timestamps.

## Additional timestamp defect found

The old query code randomized up to 16 microseconds of the request timestamp
and replaced the locally measured T1 with that randomized value. That could
introduce up to roughly 8 microseconds of offset error (half the T1 change),
which matters when interpreting differences of 7–20 microseconds. It cannot
explain the roughly 11 ms common offset or roughly 3 ms variation.

Version 0.2.2 preserves measured local T1 for the calculation, limits request
randomization to sub-60-nanosecond fraction bits, and records the wire timestamp
separately. Request/response timestamp matching remains enforced. This fix
does not turn userspace measurements into an absolute microsecond reference.

## Changes in 0.2.2

- Explicit latest-value headings, ranking estimate and ranking sample age.
- Per-source latest-versus-ranking timestamps, offsets and RTTs.
- Every score component, sample epoch and interpretation logged in decisions.
- Clear client-relative variation and advertised-reference labels.
- Original ranking weights retained; no forced preference for TS1 or the Pi.
- Physical T1 retained independently of request timestamp randomization.
- Regression coverage for component sums, mixed sample epochs, reference-ID
  neutrality and request nonce effects on measured offset.

References: [RFC 5905 packet fields and clock filtering](https://www.rfc-editor.org/rfc/rfc5905.html).
Full NTP synchronization and NTS remain separately tracked implementation work.
