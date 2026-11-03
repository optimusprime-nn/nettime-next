"""Explainable diagnostic policy; NOT RFC 5905's complete mitigation pipeline."""
from __future__ import annotations

import math
from collections import deque
from dataclasses import dataclass, field

from protocol import Sample

POLICY = "observer-consensus-v1 (experimental; not a synchronization engine)"
EVIDENCE_VERSION = 2


@dataclass
class Peer:
    host: str
    port: int = 123
    samples: deque = field(default_factory=lambda: deque(maxlen=8))
    latest: Sample | None = None
    reach: int = 0
    next_due: float = 0
    interval: float = 64
    disabled: str = ""
    recommendation: str = "Waiting for first measurement"

    @property
    def key(self):
        return f"{self.host}:{self.port}"

    def accept(self, sample):
        if self.latest and sample.address != self.latest.address:
            self.samples.clear()
        self.latest = sample
        self.reach = ((self.reach << 1) | (sample.status == "valid")) & 255
        if sample.status == "valid":
            self.samples.append(sample)

    def candidate(self, now, max_distance_ms):
        if self.disabled:
            return None, self.disabled
        if not self.latest:
            return None, "No measurement yet"
        if self.latest.status != "valid":
            return None, self.latest.reason
        usable = [s for s in self.samples if now - s.mono <= 3 * self.interval]
        if not usable:
            return None, "No fresh measurement"
        best = min(usable, key=lambda s: s.delay_ms)
        jitter = math.sqrt(sum((s.offset_ms - best.offset_ms) ** 2 for s in usable)
                           / max(1, len(usable) - 1))
        age = max(0, now - best.mono)
        precision_ms = 1000 * 2.0 ** max(-32, min(0, best.precision))
        components = dict(network_half_rtt_ms=best.delay_ms / 2,
                          upstream_half_delay_ms=max(0, best.root_delay_ms) / 2,
                          root_dispersion_ms=best.root_dispersion_ms,
                          age_allowance_ms=age * 0.015,
                          observer_precision_guard_ms=max(1, precision_ms),
                          client_offset_variation_ms=jitter)
        distance = sum(components.values())
        value = dict(peer=self.key, address=best.address, port=best.remote_port or self.port,
                     offset_ms=best.offset_ms, delay_ms=best.delay_ms,
                     stratum=best.stratum, jitter_ms=jitter, distance_ms=distance,
                     filter_samples=len(usable), sample_age_seconds=age,
                     reference_id=best.reference_id,
                     ranking_sample_utc=best.utc, ranking_sample_mono=best.mono,
                     latest_sample_utc=self.latest.utc,
                     latest_offset_ms=self.latest.offset_ms,
                     latest_delay_ms=self.latest.delay_ms,
                     distance_components_ms=components,
                     age_rate_ppm=15,
                     jitter_kind="RMS client-relative offsets around minimum-RTT sample; includes local clock drift/corrections",
                     reference_id_kind="server-advertised identifier; not hardware classification")
        if distance > max_distance_ms:
            return None, f"Estimated distance {distance:.3f} ms exceeds {max_distance_ms:g} ms limit"
        return value, "Candidate"


def recommend(peers, now, max_distance_ms=1000):
    reasons, candidates, endpoints = {}, [], set()
    for peer in peers:
        candidate, reason = peer.candidate(now, max_distance_ms)
        reasons[peer.key] = reason
        if candidate:
            endpoint = candidate["address"], candidate["port"]
            if endpoint in endpoints:
                reasons[peer.key] = "Same resolved endpoint as another entry; measured but not a separate vote"
            else:
                endpoints.add(endpoint)
                candidates.append(candidate)
    result = dict(policy=POLICY, evidence_version=EVIDENCE_VERSION, selected=None, confidence="none", reasons=reasons,
                  candidates=candidates, explanation="No eligible source; no recommendation")
    if not candidates:
        return result
    # Diagnostic interval-overlap consensus. Includes uncertainty rather than
    # choosing the fastest responder or trusting stratum alone.
    points = [c["offset_ms"] + sign * c["distance_ms"] for c in candidates for sign in (-1, 1)]
    groups = []
    for point in points:
        group = tuple(i for i, c in enumerate(candidates)
                      if abs(c["offset_ms"] - point) <= c["distance_ms"] + 1e-9)
        if group not in groups:
            groups.append(group)
    size = max(map(len, groups))
    winners = [g for g in groups if len(g) == size]
    if len(candidates) > 1 and (size <= len(candidates) / 2 or len(winners) != 1):
        for c in candidates:
            reasons[c["peer"]] = "Sources disagree or overlap is ambiguous; no unique majority"
        result["explanation"] = "No unique majority agreement; recommendation withheld"
        return result
    survivors = [candidates[i] for i in winners[0]]
    chosen = min(survivors, key=lambda c: (c["distance_ms"], c["jitter_ms"], c["stratum"], c["peer"]))
    for c in candidates:
        if c not in survivors:
            reasons[c["peer"]] = "Outside majority agreement interval; still monitored"
        elif c is not chosen:
            reasons[c["peer"]] = (f"Agrees; distance {c['distance_ms']:.3f} ms; "
                                   f"recommended source has {chosen['distance_ms']:.3f} ms")
        else:
            reasons[c["peer"]] = (f"Recommended: lowest estimated distance among agreeing sources "
                                   f"({c['distance_ms']:.3f} ms; jitter {c['jitter_ms']:.3f} ms; "
                                   f"stratum {c['stratum']}; ranking sample {c['sample_age_seconds']:.1f}s old)")
    result.update(selected=chosen["peer"],
                  confidence="single source / uncorroborated" if len(candidates) == 1 else "endpoint agreement / independence unverified",
                  explanation=reasons[chosen["peer"]])
    return result
