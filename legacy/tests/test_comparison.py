import json
from pathlib import Path
import tempfile
import unittest

from comparison import build_report, export_report
from protocol import Sample
from storage import Store


class ComparisonTests(unittest.TestCase):
    def test_window_statistics_authentication_stratum_changes_and_selection_expiry(self):
        with tempfile.TemporaryDirectory() as folder:
            store = Store(Path(folder) / 'history.sqlite3')
            for cycle, (stamp, offset, stratum, status) in enumerate(((1000, -1, 1, 'valid'), (1064, 3, 2, 'valid'), (1128, None, None, 'timeout'))):
                s = Sample('fixture', 123, stamp, cycle, status=status, offset_ms=offset, delay_ms=10 if offset is not None else None,
                           stratum=stratum, authentication_status='authenticated')
                d = dict(selected='fixture:123' if cycle < 2 else None, confidence='test', explanation='fixture',
                         reasons={'fixture:123': 'Recommended: fixture'}, servers=[dict(peer='fixture:123', interval_seconds=64)])
                store.save_cycle(cycle, [s], d, {}, stamp)
            report = build_report(store, 7, now=2000)
            row = report['servers'][0]
            self.assertEqual(row['attempts'], 3)
            self.assertAlmostEqual(row['valid_percent'], 200 / 3)
            self.assertEqual(row['median_offset_ms'], 1)
            self.assertEqual(row['authenticated_valid'], 2)
            self.assertEqual(row['stratum_changes'], 1)
            self.assertAlmostEqual(row['selected_hours'], 128 / 3600)
            self.assertEqual(len(report['selection_changes']), 2)
            export_report(report, folder)
            self.assertEqual(json.loads((Path(folder) / 'comparison.json').read_text())['servers'][0]['attempts'], 3)
            self.assertTrue((Path(folder) / 'selection-changes.csv').is_file())
            # A long last selection doesn't turn downtime into selected uptime.
            with store.connect() as db:
                db.execute('DELETE FROM decisions WHERE utc=1128')
            report = build_report(store, 7, now=2000)
            self.assertAlmostEqual(report['servers'][0]['selected_hours'], (64 + 192) / 3600)

    def test_carry_in_selection_counts_only_fresh_overlap_and_future_data_is_excluded(self):
        with tempfile.TemporaryDirectory() as folder:
            store = Store(Path(folder) / 'history.sqlite3')
            d = dict(selected='fixture:123', confidence='test', explanation='carry', servers=[], reasons={})
            store.save_cycle(1, [], d, {}, 1000)
            store.save_cycle(2, [Sample('future', 123, 999999, 0)], d, {}, 999999)
            report = build_report(store, days=100 / 86400, now=1150)
            self.assertEqual(report['servers'], [])
            self.assertEqual(report['selection_changes'], [])

    def test_empty_store_exports_stable_empty_reports(self):
        with tempfile.TemporaryDirectory() as folder:
            report = build_report(Store(Path(folder) / 'history.sqlite3'))
            export_report(report, folder)
            self.assertEqual(report['servers'], [])
            self.assertIn('peer', (Path(folder) / 'comparison.csv').read_text(encoding='utf-8-sig'))
