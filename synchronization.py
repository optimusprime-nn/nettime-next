"""NTP mitigation assessment, separate from the legacy diagnostic ranking.

RFC 5905 §§10–11: eight-stage filtering, aged dispersion, intersection,
clustering and inverse-distance combination. Conservative local safeguards
require multiple explicitly grouped sources and a filled-enough filter.
This preview does not claim complete RFC 5905 conformance.
"""
import math

POLICY = "ntp-mitigation-preview-v1"
PHI_MS_PER_SECOND = .015
LOCAL_PRECISION_MS = 1.0


def filtered(peer, now, max_distance_ms):
    if peer.disabled or not peer.latest or peer.latest.status != 'valid' or not peer.reach:
        return None, peer.disabled or 'No current valid response'
    if now - peer.latest.mono > 3 * peer.interval:
        return None, 'Latest response expired'
    samples = [s for s in peer.samples if 0 <= now - s.mono <= 8 * peer.interval + 1]
    if not samples:
        return None, 'Filter expired'
    samples.sort(key=lambda s: (s.delay_ms, -s.mono))
    best = samples[0]
    if best.leap not in (0, 1, 2) or not 1 <= best.stratum <= 15:
        return None, 'Unsynchronized source'
    if best.stratum > 1 and best.reference_id == best.local_address:
        return None, 'Synchronization loop: upstream references this computer'
    dispersion = 0
    for i in range(8):
        if i < len(samples):
            s = samples[i]
            precision = 1000 * 2.0 ** max(-32, min(0, s.precision))
            epsilon = LOCAL_PRECISION_MS + precision + PHI_MS_PER_SECOND * (max(0, s.exchange_ms or 0) / 1000 + now - s.mono)
        else:
            epsilon = 16000  # Unfilled stages start with MAXDISP (16 seconds).
        dispersion += epsilon / 2 ** (i + 1)
    jitter = max(LOCAL_PRECISION_MS, math.sqrt(sum((s.offset_ms - best.offset_ms) ** 2 for s in samples) / max(1, len(samples) - 1)))
    components = dict(half_network_delay_ms=best.delay_ms / 2,
                      half_root_delay_ms=max(0, best.root_delay_ms) / 2,
                      root_dispersion_ms=best.root_dispersion_ms,
                      filter_dispersion_ms=dispersion, jitter_ms=jitter)
    distance = max(LOCAL_PRECISION_MS, sum(components.values()))
    if distance > max_distance_ms:
        return None, f'Root distance {distance:.3f} ms exceeds limit; filter warming or source uncertain'
    return dict(peer=peer.key, address=best.address, port=best.remote_port or peer.port,
                offset_ms=best.offset_ms, jitter_ms=jitter, distance_ms=distance,
                distance_components_ms=components, stratum=best.stratum, leap=best.leap,
                sample_mono=best.mono, sample_utc=best.utc, samples=len(samples),
                authentication=best.authentication_status, interval_seconds=peer.interval), 'Filtered candidate'


def intersection(candidates):
    """RFC 5905 §11.2.1 midpoint-aware correctness-interval search."""
    endpoints = sorted((c['offset_ms'] + kind * c['distance_ms'], kind)
                       for c in candidates for kind in (-1, 0, 1))
    m = len(candidates)
    for falsetickers in range((m - 1) // 2 + 1):
        found, count = 0, 0
        low, high = math.inf, -math.inf
        for edge, kind in endpoints:
            count -= kind
            if count >= m - falsetickers:
                low = edge
                break
            if kind == 0:
                found += 1
        count = 0
        for edge, kind in reversed(endpoints):
            count += kind
            if count >= m - falsetickers:
                high = edge
                break
            if kind == 0:
                found += 1
        if found <= falsetickers and low < high:
            return low, high, falsetickers
    return None


def assess(peers, cfg, now):
    result = dict(policy=POLICY, ready=False, selected=None, offset_ms=None, jitter_ms=None,
                  candidates=[], survivors=[], reasons={}, cluster_rounds=[], explanation='Filters warming; no clock correction')
    options = cfg.get('clock', {})
    minimum = options.get('min_sources', 3)
    configs = {f"{s['host']}:{s['port']}": s for s in cfg['servers']}
    endpoints, groups = set(), set()
    for peer in peers:
        candidate, reason = filtered(peer, now, cfg['max_distance_ms'])
        result['reasons'][peer.key] = reason
        if candidate is None:
            continue
        group = configs[peer.key].get('source_group', '').strip()
        endpoint = candidate['address'], candidate['port']
        candidate['source_group'] = group
        if endpoint in endpoints:
            result['reasons'][peer.key] = 'Same resolved endpoint; no additional vote'
        elif not group:
            result['reasons'][peer.key] = 'Assign an independent source group before synchronization'
        elif group in groups:
            result['reasons'][peer.key] = 'Same source group; measured but no additional vote'
        elif candidate['samples'] < 4:
            result['reasons'][peer.key] = 'Filter warming: need at least four fresh measurements'
        elif options.get('require_nts', False) and candidate['authentication'] != 'authenticated':
            result['reasons'][peer.key] = 'Clock policy requires NTS authentication'
        else:
            endpoints.add(endpoint)
            groups.add(group)
            result['candidates'].append(candidate)
    candidates = result['candidates']
    if len(candidates) < minimum:
        result['explanation'] = f'Need {minimum} eligible source groups; currently {len(candidates)}'
        return result
    bounds = intersection(candidates)
    if bounds is None:
        result['explanation'] = 'No majority correctness interval; synchronization withheld'
        for c in candidates:
            result['reasons'][c['peer']] = result['explanation']
        return result
    low, high, falsetickers = bounds
    result['intersection_ms'] = [low, high]
    result['allowed_falsetickers'] = falsetickers
    survivors = [c for c in candidates if low <= c['offset_ms'] <= high]
    for c in candidates:
        if c not in survivors:
            result['reasons'][c['peer']] = 'Falseticker: midpoint outside majority correctness interval'
    survivors.sort(key=lambda c: (c['stratum'] * cfg['max_distance_ms'] + c['distance_ms'], c['peer']))
    selection_jitter = 0
    while survivors:
        jitters = [math.sqrt(sum((c['offset_ms'] - q['offset_ms']) ** 2 for q in survivors) / max(1, len(survivors) - 1)) for c in survivors]
        worst = max(range(len(survivors)), key=lambda i: jitters[i])
        selection_jitter = jitters[worst]
        if len(survivors) <= minimum or selection_jitter < min(c['jitter_ms'] for c in survivors):
            break
        removed = survivors.pop(worst)
        result['cluster_rounds'].append(dict(removed=removed['peer'], selection_jitter_ms=selection_jitter))
        result['reasons'][removed['peer']] = 'Cluster outlier: largest selection jitter'
    result['survivors'] = survivors
    if len(survivors) < minimum:
        result['explanation'] = f'Only {len(survivors)} survivors after mitigation; need {minimum}'
        return result
    leaps = {c['leap'] for c in survivors}
    if leaps != {0}:
        result['explanation'] = 'Leap announcement/conflict: correction suspended in this preview'
        return result
    weight = sum(1 / c['distance_ms'] for c in survivors)
    offset = sum(c['offset_ms'] / c['distance_ms'] for c in survivors) / weight
    chosen = survivors[0]
    for c in survivors:
        result['reasons'][c['peer']] = ('System peer' if c is chosen else 'Combined survivor') + f"; weight {(1 / c['distance_ms']) / weight:.6f}"
    result.update(ready=True, selected=chosen['peer'], offset_ms=offset,
                  jitter_ms=math.hypot(selection_jitter, chosen['jitter_ms']),
                  sample_mono=chosen['sample_mono'], sample_utc=chosen['sample_utc'],
                  interval_seconds=chosen['interval_seconds'],
                  explanation=f"{len(survivors)} source groups survived; combined offset {offset:+.3f} ms; system peer {chosen['peer']}")
    return result
