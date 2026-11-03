# Standards conformance matrix

This maps the parts of RFC 5905 (NTPv4) and RFC 8915 (NTS) that NetTime Next
implements to the code that implements them and the tests that check them. It
covers the native application in `native/`. The legacy Python implementation
in `legacy/` is not covered.

NetTime Next does **not** claim full RFC 5905 conformance. It implements a
deliberately narrow profile, listed below, and replaces the RFC clock
discipline with its own bounded, slew-only controller. Rows marked **Partial**
or **Gap** are open work. Rows marked **Not in profile** are out of scope by
design.

Test names refer to `#[test]` functions in the named source file, or in
`native/tests/` where stated. Run them with
`cargo test --manifest-path native/Cargo.toml --locked`.

## Supported profile

- Unicast **client** associations only (mode 3 requests, mode 4 replies).
- One request per source per poll. No symmetric, broadcast, manycast or server
  modes, and no interleaved mode.
- Optional NTS (RFC 8915) per source. A source marked NTS-required never falls
  back to plain NTP.
- Clock control by gradually changing the Windows clock rate. The clock is
  never stepped.

## RFC 5905: NTPv4

| Requirement | RFC section | Status | Implementation | Tests |
| --- | --- | --- | --- | --- |
| 64-bit timestamp format and era handling | §6 | Done | `protocol.rs` `encode`, `decode` (era resolved against a pivot time) | `protocol.rs` `era_round_trip` |
| Client request with unpredictable origin timestamp | §8, §9.1 | Done | `protocol.rs` request build | `protocol.rs` `origin_timestamp_is_unpredictable_but_close` |
| Reply sanity: origin match (replay/bogus), server mode, version 3 or 4, local clock unchanged during the exchange | §8 (tests 1–3) | Done | `protocol.rs` reply checks | `protocol.rs` `replay_and_unsynchronized_rejected`, `stray_datagrams_are_discarded_not_fatal`, `only_stray_datagrams_time_out_with_their_reason` |
| Reject unsynchronized servers (leap 3, stratum 0 or above 15) | §8, §9.2 | Done | `protocol.rs` validity check | `protocol.rs` `replay_and_unsynchronized_rejected` |
| Offset and delay calculation; missing or misordered server timestamps and impossible negative delays rejected | §8 | Done | `protocol.rs` (fixed 1 ms rounding allowance; server precision cannot widen it) | `protocol.rs` `coarse_server_precision_does_not_allow_large_negative_delay` |
| Kiss-o'-death: RATE backs off, DENY/RSTR stop polling | §7.4 | Done | `selection.rs` `Peer::accept` | `selection.rs` `rate_kiss_of_death_backoff_is_bounded`, `rate_backoff_unwinds_after_steady_replies`, `kiss_of_death_removes_a_source_at_once` |
| Reach register | §13 | Done | `selection.rs` `Peer::accept` | `selection.rs` `one_lost_reply_does_not_remove_a_source` |
| Clock filter: eight-stage register, minimum-delay sample, aged dispersion, missing stages at MAXDISP, RMS jitter | §10 | Done | `selection.rs` `filtered` | `selection.rs` `stale_or_unfilled_filters_cannot_control`; `native/tests/parity.rs` |
| Clock filter: popcorn spike suppressor | §10 | Gap | Not implemented | None |
| Do not reuse an already-used filter sample | §10 | Done | `clock.rs` ("not reused") | `clock.rs` `controller_warms_deduplicates_bounds_and_restores_on_conflict` |
| Root distance and distance threshold | §10, §11.2 | Done | `selection.rs` `filtered` (configurable `max_distance_ms`) | `selection.rs` `stale_or_unfilled_filters_cannot_control` |
| Synchronization loop detection (reference ID equals local address) | §11.2 | Done | `selection.rs` `filtered` | None |
| Selection: intersection algorithm, falseticker rejection | §11.2.1 | Done | `selection.rs` `intersection`, `assess` | `selection.rs` `falseticker_cannot_move_combined_time`; `native/tests/parity.rs` |
| Clustering: drop the survivor with the largest selection jitter until the minimum is reached | §11.2.2 | Done | `selection.rs` `assess` | `native/tests/parity.rs` |
| Combine: offsets weighted by inverse root distance; system jitter | §11.2.3 | Done | `selection.rs` `assess` | `selection.rs` `agreement_combines_independent_groups`; `native/tests/parity.rs` |
| System peer: lowest stratum, then distance | §11.2.3 | Done | `selection.rs` `assess` | `selection.rs` `group_votes_through_its_best_source_not_the_first_configured` |
| Minimum survivors (NMIN) | §11.2 | Done (stricter) | `clock.min_sources`, at least 3, counted by independent source group | `selection.rs` `same_group_cannot_multiply_votes`; `config.rs` `release_defaults_have_public_authenticated_groups_and_do_not_take_over` |
| Leap second announcements | §9.2, §11.3 | Partial | Correction suspends while any survivor announces a leap. The leap itself is left to Windows. | `clock.rs` `stale_excessive_leap_and_holdover_suspend` |
| Clock discipline: PLL/FLL hybrid, step threshold, stepout | §11.3 | Not in profile | Replaced by the bounded slew controller (see below) | See below |
| Poll process: adaptive poll interval | §13 | Gap | Fixed configured interval (64 s default). Only RATE kiss-o'-death changes it. | `selection.rs` `rate_kiss_of_death_backoff_is_bounded` |
| Symmetric, broadcast, manycast and server modes | §9, §15 | Not in profile | Not implemented | None |

## RFC 8915: Network Time Security

| Requirement | RFC section | Status | Implementation | Tests |
| --- | --- | --- | --- | --- |
| NTS-KE over TLS 1.3 with ALPN `ntske/1` and certificate verification | §3, §4 | Done | `nts.rs` `establish` (rustls, TLS 1.3 only, Mozilla root bundle via `webpki-roots`, host-name identity) | `protocol.rs` `live_nts_session_reuse` (ignored by default; needs network) |
| NTS-KE record parsing: critical bit, end record, error/warning records, duplicates, size limit | §4 | Done | `nts.rs` `establish` | None in unit tests; exercised by fuzzing (not committed) |
| AEAD algorithm 15 (AES-SIV-CMAC-256) | §4.1.5, §5.6 | Done | `nts.rs` (`Aes128Siv` with a 256-bit key) | `nts.rs` `aes_siv_matches_python_interoperability_vector` |
| Key export (`EXPORTER-network-time-security`, C2S/S2C) | §5.1 | Done | `nts.rs` `establish` | `protocol.rs` `live_nts_session_reuse` |
| Negotiated NTPv4 server and port records | §4.1.7, §4.1.8 | Done | `nts.rs` `establish` | None |
| Overall NTS-KE time limit | §4 (operational) | Done | `nts.rs` `Deadline` | `nts.rs` `trickling_key_exchange_peer_cannot_outlast_the_deadline` |
| Request extensions: unique identifier, cookie, placeholders, authenticator | §5.3–§5.7 | Done | `nts.rs` request build | `protocol.rs` `live_nts_session_reuse` |
| Reply verification: one authenticator, matching unique ID, no plaintext cookies, AEAD decrypt | §5.7, §5.8 | Done | `nts.rs` `Session::verify` | `nts.rs` `authentic_reply_replenishes_cookies`, `forged_or_malformed_replies_are_rejected_without_taking_cookies` |
| Extension field framing and length checks | §5.2, RFC 7822 | Done | `nts.rs` `fields` | `nts.rs` `rejects_truncated_and_misaligned_extensions` |
| Cookie pool (up to 8), single use, replenishment | §5.7, §6 | Done | `nts.rs` | `protocol.rs` `lost_nts_reply_keeps_session_until_repeated`, `live_nts_session_reuse` |
| Re-key after repeated failures, bounded back-off | §6 (operational) | Done | `nts.rs` (`MAX_MISSES`, `MAX_BACKOFF`) | `protocol.rs` `nts_backoff_is_bounded`, `polls_during_nts_backoff_do_not_extend_it` |
| No downgrade to plain NTP | §1.2 | Done | `nts.rs`, `protocol.rs` | `selection.rs` `nts_required_policy_rejects_unauthenticated_votes` |
| Session keys and cookies kept out of logs and storage | §8 (security) | Done | Zeroized buffers; keys never persisted | None |

## Clock control (in place of RFC 5905 §11.3)

| Behaviour | Status | Implementation | Tests |
| --- | --- | --- | --- |
| Correct by changing the clock rate; never step | Done | `clock.rs` | `clock.rs` `closed_loop_converges_for_both_oscillator_directions` |
| Rate bounded by `max_slew_ppm` | Done | `clock.rs` | `clock.rs` `controller_warms_deduplicates_bounds_and_restores_on_conflict` |
| Correct direction and quantization of the Windows rate | Done | `clock.rs` | `clock.rs` `frequency_direction_and_quantization_are_correct` |
| Frequency estimate that tracks drift and ignores system-peer switches | Done | `clock.rs` | `clock.rs` `frequency_estimate_tracks_a_drifting_clock_with_old_samples`, `system_peer_switch_within_one_poll_round_does_not_kick_frequency` |
| Large offsets recover gradually after three fresh agreeing samples, up to a limit | Done | `clock.rs` | `clock.rs` `large_offsets_recover_in_both_directions_and_survive_an_outage`, `recovery_confirms_new_samples_and_stops_on_lost_agreement`, `recovery_rejects_stale_evidence_and_can_be_disabled_by_its_limit`, `leaving_normal_tracking_requires_fresh_recovery_confirmation` |
| Holdover limit, then suspend | Done | `clock.rs` | `clock.rs` `stale_excessive_leap_and_holdover_suspend` |
| Rate read back from Windows must match the request within 1 ppm | Done | `clock.rs` | Runs live; no unit test (needs the real API) |
| Actual clock response measured against the performance counter | Done, CI only | `validation.rs` `clock_response` | Disposable runners only; not yet run against 1.0.1 |
| Exclusive ownership, takeover and exact rollback | Done | `ownership.rs` | `ownership.rs` (7 tests) |

## Open gaps

1. Popcorn spike suppressor in the clock filter.
2. Adaptive poll interval.
3. Unit tests for NTS-KE record parsing.
4. Interoperability comparison against an independent client, such as chrony
   tracking the same sources, over at least a day.
5. Coverage-guided fuzzing of the NTP and NTS parsers. A no-download fuzz pass of
   about 4.3 million cases found no panics, but it is not part of the repository.
6. Physical clock-response and install/upgrade tests in a disposable Windows VM.
