import copy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock

from clock_control import Controller, SlewPlanner
from monitor import validate_config
from protocol import Sample
from selection import Peer
from synchronization import assess, filtered, intersection


def population(offsets=(1, 1.1, .9), count=8, now=1000):
    cfg = validate_config({'servers': [dict(host=f's{i}', source_group=f'group{i}') for i in range(len(offsets))]})
    peers = []
    for i, offset in enumerate(offsets):
        peer = Peer(f's{i}')
        for j in range(count):
            stamp = now - (count - j - 1) * 64
            peer.accept(Sample(peer.host, 123, 1792190000 + stamp, stamp, status='valid', address=f'192.0.2.{i + 1}',
                stratum=2, leap=0, precision=-20, root_delay_ms=0, root_dispersion_ms=1, delay_ms=10,
                offset_ms=offset, exchange_ms=10, local_address='192.0.2.254'))
        peers.append(peer)
    return cfg, peers


class MitigationTests(unittest.TestCase):
    def test_filter_ages_dispersion_and_keeps_eight_stages(self):
        cfg, peers = population()
        a, _ = filtered(peers[0], 1000, 1000)
        b, _ = filtered(peers[0], 1064, 1000)
        self.assertEqual(a['samples'], 8)
        self.assertGreater(b['distance_ms'], a['distance_ms'])
        self.assertIsNone(filtered(peers[0], 1300, 1000)[0])

    def test_unfilled_filter_is_not_silently_treated_as_precise(self):
        cfg, peers = population(count=1)
        self.assertFalse(assess(peers, cfg, 1000)['ready'])
        self.assertIsNone(filtered(peers[0], 1000, 1000)[0])

    def test_combined_offset_and_each_weight_are_logged(self):
        cfg, peers = population()
        result = assess(peers, cfg, 1000)
        self.assertTrue(result['ready'], result)
        self.assertAlmostEqual(result['offset_ms'], 1, places=6)
        self.assertEqual(len(result['survivors']), 3)
        self.assertTrue(all('weight' in result['reasons'][p.key] for p in peers))

    def test_falseticker_and_tied_split_are_rejected(self):
        cfg, peers = population((1, 1.1, .9, 1000))
        result = assess(peers, cfg, 1000)
        self.assertTrue(result['ready'], result)
        self.assertIn('Falseticker', result['reasons']['s3:123'])
        cfg, peers = population((0, 0, 100, 100))
        self.assertFalse(assess(peers, cfg, 1000)['ready'])

    def test_duplicate_endpoint_provider_and_unassigned_groups_never_add_votes(self):
        for kind in ('endpoint', 'group', 'unassigned'):
            cfg, peers = population()
            if kind == 'endpoint':
                for s in peers[1].samples:
                    s.address = peers[0].latest.address
            elif kind == 'group':
                cfg['servers'][1]['source_group'] = cfg['servers'][0]['source_group']
            else:
                cfg['servers'][1]['source_group'] = ''
            self.assertFalse(assess(peers, cfg, 1000)['ready'], kind)

    def test_leap_loop_failure_and_auth_policy_suspend(self):
        for kind in ('leap', 'loop', 'failure', 'nts'):
            cfg, peers = population()
            if kind == 'leap':
                for p in peers:
                    for s in p.samples:
                        s.leap = 1
            elif kind == 'loop':
                for s in peers[0].samples:
                    s.reference_id = s.local_address
            elif kind == 'failure':
                peers[0].accept(Sample('s0', 123, 0, 1000, status='timeout'))
            else:
                cfg['clock']['require_nts'] = True
            self.assertFalse(assess(peers, cfg, 1000)['ready'], kind)

    def test_cluster_outlier_rounds_are_recorded(self):
        cfg, peers = population((0, .1, .2, .3, 10))
        for p in peers:
            for s in p.samples:
                s.root_dispersion_ms = 50
        result = assess(peers, cfg, 1000)
        self.assertTrue(result['ready'])
        self.assertEqual(result['cluster_rounds'][0]['removed'], 's4:123')
        self.assertLess(abs(result['offset_ms']), 1)


class DisciplineTests(unittest.TestCase):
    def options(self):
        return validate_config({'servers': [{'host': 'fixture'}]})['clock']

    def assessment(self, stamp, offset):
        return dict(ready=True, sample_mono=stamp, offset_ms=offset, explanation='fixture')

    def test_three_new_samples_required_and_old_sample_never_reused(self):
        planner = SlewPlanner(self.options())
        self.assertEqual(planner.plan(self.assessment(0, 10), 0)['action'], 'warmup')
        self.assertEqual(planner.plan(self.assessment(0, 10), 1)['action'], 'hold')
        self.assertEqual(planner.updates, 1)
        planner.plan(self.assessment(64, 10), 64)
        self.assertEqual(planner.plan(self.assessment(128, 10), 128)['action'], 'slew')

    def test_simulated_fast_and_slow_clocks_converge_without_steps(self):
        for drift in (-40, 35):
            planner = SlewPlanner(self.options())
            offset, rate = 80., 0.
            for tick in range(600):
                now = tick * 64
                plan = planner.plan(self.assessment(now, offset), now)
                rate = plan['rate_ppm']
                self.assertLessEqual(abs(rate), 250)
                offset += (drift - rate) * 64 / 1000
            self.assertLess(abs(offset), .1, (drift, offset))

    def test_large_errors_no_consensus_expiry_and_external_conflict_restore_rate(self):
        backend, store = Mock(), Mock()
        controller = Controller(self.options(), store, backend, lambda: True)
        for i in range(3):
            controller.update(self.assessment(i * 64, 10), i * 64)
        backend.rate.assert_called()
        self.assertEqual(controller.update(self.assessment(192, 200), 192)['action'], 'suspend')
        controller.update(dict(ready=False, explanation='No majority'), 193)
        backend.restore.assert_called()
        controller.planner.last_good = 100
        controller.suspended = False
        controller.watchdog(1001)
        self.assertTrue(controller.suspended)
        controller.guard = lambda: False
        with self.assertRaises(RuntimeError):
            controller.watchdog(1004)

    def test_invalid_limits_cannot_enable_unbounded_adjustments(self):
        for key, value in (('max_offset_ms', 129), ('max_slew_ppm', 501), ('holdover_seconds', 3601), ('min_sources', 1)):
            with self.assertRaises(ValueError):
                validate_config({'servers': [{'host': 'fixture'}], 'clock': {key: value}})
