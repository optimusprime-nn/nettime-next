"""Measure actuator direction/gain on a disposable Windows CI runner only.

This is not a readback test: compare wall-clock progress against QPC while
applying both signs. No NTP fixture is allowed to control the clock here.
"""
import json
import os
from pathlib import Path
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))


def response(seconds=20):
    from protocol import utc_now
    start = time.perf_counter()
    readings = []
    while time.perf_counter() - start < seconds:
        before = time.perf_counter()
        wall = utc_now()
        after = time.perf_counter()
        if after - before < .001:
            readings.append(((before + after) / 2, wall))
        time.sleep(.05)
    if len(readings) < 100:
        raise RuntimeError('Insufficient precise clock observations')
    m0, w0 = readings[0]
    xy = [(m - m0, (w - w0) - (m - m0)) for m, w in readings]
    mx = sum(x for x, _ in xy) / len(xy)
    my = sum(y for _, y in xy) / len(xy)
    slope = sum((x - mx) * (y - my) for x, y in xy) / sum((x - mx) ** 2 for x, _ in xy)
    return slope * 1000000


def main():
    if os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted':
        raise RuntimeError('Clock-response test is restricted to disposable GitHub-hosted runners')
    from takeover import WindowsServices, CONTROLLERS
    from runtime_paths import SERVICE_NAME
    from clock_control import WindowsClock
    backend = WindowsServices()
    assert not any(backend.running(name) for name in (*CONTROLLERS, SERVICE_NAME)), 'A clock service is still running'
    clock = WindowsClock()
    records = []
    try:
        for target in (0., 200., -200.):
            readback = clock.rate(target)
            time.sleep(1)
            observed = response()
            record = dict(target_wall_rate_ppm=target, readback_wall_rate_ppm=readback,
                          observed_wall_vs_qpc_ppm=observed)
            print(json.dumps(record), flush=True)
            records.append(record)
        baseline = records[0]['observed_wall_vs_qpc_ppm']
        for record in records[1:]:
            effective = record['observed_wall_vs_qpc_ppm'] - baseline
            record['observed_relative_to_baseline_ppm'] = effective
            assert abs(effective - record['target_wall_rate_ppm']) < 30, record
    finally:
        clock.restore()
        output = Path(__file__).resolve().parents[1] / 'dist' / 'clock-response.json'
        output.parent.mkdir(exist_ok=True)
        output.write_text(json.dumps(records, indent=2) + '\n', encoding='utf-8')
    print('PASS: actual wall-clock response has correct direction and gain for both correction signs', flush=True)


if __name__ == '__main__':
    main()
