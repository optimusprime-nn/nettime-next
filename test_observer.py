import copy
import json
import socket
import struct
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest.mock import patch
from unittest.mock import MagicMock

from monitor import InstanceLock, Monitor, control_state, load_config, validate_config
from protocol import InvalidReply, KissOfDeath, Sample, decode_time, encode_time, parse_reply, query
from selection import Peer, recommend
from storage import Store

TEST_ROOT = Path(__file__).resolve().parents[2] / "work" / "tests"
TEST_ROOT.mkdir(parents=True, exist_ok=True)


def fixture():
    t1 = 1792190000.0
    stamp = encode_time(t1)
    raw = bytearray(48)
    raw[0], raw[1], raw[2], raw[3] = 36, 2, 6, (-20) & 255
    raw[8:12] = struct.pack("!I", 65)
    raw[12:16] = socket.inet_aton("192.0.2.1")
    raw[16:24] = encode_time(t1 - 30)
    raw[24:32], raw[32:40], raw[40:48] = stamp, encode_time(t1 + .010), encode_time(t1 + .011)
    sample = Sample("fixture", 123, t1, 100, address="192.0.2.2", t1=t1, t4=t1 + .021,
                    clock_step_ms=0, exchange_ms=21)
    return raw, stamp, sample


def peer(host, offset=0, delay=10, stratum=2, root_dispersion=1, address=None):
    p = Peer(host)
    sample = Sample(host, 123, 1792190000, 100, status="valid", address=address or host,
        stratum=stratum, precision=-20, root_delay_ms=0, root_dispersion_ms=root_dispersion,
        offset_ms=offset, delay_ms=delay, reason="valid")
    p.accept(sample)
    return p


class ProtocolTests(unittest.TestCase):
    def test_request_randomization_does_not_replace_measured_t1(self):
        raw, _, template = fixture()
        sent = []
        sock = MagicMock()
        sock.__enter__.return_value = sock
        sock.getsockname.return_value = ("192.0.2.2", 55000)
        sock.send.side_effect = lambda packet: sent.append(bytes(packet))
        def response(size):
            reply = bytearray(raw)
            reply[24:32] = sent[0][40:48]
            return bytes(reply)
        sock.recv.side_effect = response
        with patch("protocol.socket.getaddrinfo", return_value=[(socket.AF_INET, socket.SOCK_DGRAM, 17, "", ("192.0.2.1", 123))]), \
             patch("protocol.socket.socket", return_value=sock), \
             patch("protocol.utc_now", side_effect=[template.t1, template.t1, template.t4]), \
             patch("protocol.time.perf_counter", side_effect=[100, 100, 100.021]), \
             patch("protocol.secrets.token_bytes", return_value=b"\xff") as random:
            sample = query("test", 123, 1, 64)
        self.assertEqual(sample.status, "valid", sample.reason)
        self.assertEqual(sample.t1, template.t1)
        self.assertAlmostEqual(sample.offset_ms, 0, places=3)
        random.assert_called_once_with(1)
        self.assertNotEqual(sent[0][40:48], encode_time(template.t1))

    def test_four_timestamp_calculation(self):
        raw, stamp, sample = fixture()
        result = parse_reply(raw, stamp, sample)
        self.assertEqual(result.stratum, 2)
        self.assertAlmostEqual(result.delay_ms, 20, places=2)
        self.assertAlmostEqual(result.offset_ms, 0, places=2)

    def test_2036_era_rollover(self):
        for unix in (2085978495.125, 2085978496.125, 2200000000.5):
            self.assertAlmostEqual(decode_time(encode_time(unix), unix), unix, places=5)

    def test_invalid_headers(self):
        for index, value, reason in ((0, 35, "mode"), (0, 12, "version"),
                                     (0, 228, "unsynchronized"), (1, 16, "stratum")):
            with self.subTest(reason=reason):
                raw, stamp, sample = fixture()
                raw[index] = value
                with self.assertRaisesRegex(InvalidReply, reason):
                    parse_reply(raw, stamp, sample)

    def test_truncated_header(self):
        raw, stamp, sample = fixture()
        with self.assertRaisesRegex(InvalidReply, "Truncated"):
            parse_reply(raw[:40], stamp, sample)

    def test_originate_mismatch(self):
        raw, stamp, sample = fixture()
        raw[24] ^= 1
        with self.assertRaisesRegex(InvalidReply, "Originate"):
            parse_reply(raw, stamp, sample)

    def test_missing_timestamps(self):
        for index in (16, 32, 40):
            raw, stamp, sample = fixture()
            raw[index:index + 8] = bytes(8)
            with self.assertRaisesRegex(InvalidReply, "Missing"):
                parse_reply(raw, stamp, sample)

    def test_no_silent_acceptance_of_authentication(self):
        raw, stamp, sample = fixture()
        with self.assertRaisesRegex(InvalidReply, "unsupported"):
            parse_reply(raw + bytes(20), stamp, sample)

    def test_server_time_order(self):
        raw, stamp, sample = fixture()
        raw[40:48] = encode_time(sample.t1)
        with self.assertRaisesRegex(InvalidReply, "precedes"):
            parse_reply(raw, stamp, sample)

    def test_impossible_delay(self):
        raw, stamp, sample = fixture()
        raw[40:48] = encode_time(sample.t1 + 1)
        with self.assertRaisesRegex(InvalidReply, "negative"):
            parse_reply(raw, stamp, sample)

    def test_clock_step(self):
        raw, stamp, sample = fixture()
        sample.clock_step_ms = 25
        with self.assertRaisesRegex(InvalidReply, "Local clock changed"):
            parse_reply(raw, stamp, sample)

    def test_kiss_of_death_validates_origin(self):
        raw, stamp, sample = fixture()
        raw[1], raw[12:16] = 0, b"RATE"
        with self.assertRaises(KissOfDeath) as raised:
            parse_reply(raw, stamp, sample)
        self.assertEqual(raised.exception.code, "RATE")
        raw[24] ^= 1
        with self.assertRaisesRegex(InvalidReply, "Originate"):
            parse_reply(raw, stamp, sample)

    def test_nts_required_never_sends_plaintext(self):
        with patch("nts.establish", side_effect=ValueError("Certificate verification failed")), patch("protocol.socket.getaddrinfo") as dns, patch("protocol.socket.socket") as sock:
            result = query("example.invalid", 123, 1, 64, security_mode="nts_required")
            self.assertEqual(result.status, "security_blocked")
            self.assertEqual(result.authentication_status, "failed")
            dns.assert_not_called()
            sock.assert_not_called()

    def test_local_udp_interoperability_and_ephemeral_port(self):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as server:
            server.bind(("127.0.0.1", 0))
            server.settimeout(2)
            def respond():
                request, remote = server.recvfrom(512)
                now = time.time()
                raw = bytearray(48)
                raw[0], raw[1], raw[3] = 36, 1, (-10) & 255
                raw[12:16] = b"GPS\0"
                raw[16:24] = encode_time(now - 1)
                raw[24:32] = request[40:48]
                raw[32:40] = raw[40:48] = encode_time(now)
                server.sendto(raw, remote)
            worker = threading.Thread(target=respond)
            worker.start()
            result = query("127.0.0.1", server.getsockname()[1], 2, 64)
            worker.join(3)
            self.assertEqual(result.status, "valid", result.reason)
            self.assertNotEqual(result.source_port, 123)
            self.assertEqual(result.reference_id, "GPS")


class SelectionTests(unittest.TestCase):
    def test_ranking_sample_and_latest_measurement_are_distinct_and_explained(self):
        from app import ranking_evidence
        p = peer("ts1", offset=-7.159114, delay=.141382, root_dispersion=.091553)
        p.latest.utc = 1000
        latest = copy.deepcopy(p.latest)
        latest.offset_ms, latest.delay_ms, latest.mono, latest.utc = -10.944605, .171185, 229, 1129
        p.accept(latest)
        candidate, _ = p.candidate(229.1350751, 1000)
        parts = candidate["distance_components_ms"]
        self.assertAlmostEqual(parts["age_allowance_ms"], 1.9370261265)
        self.assertAlmostEqual(sum(parts.values()), candidate["distance_ms"])
        self.assertEqual(candidate["ranking_sample_utc"], 1000)
        self.assertEqual(candidate["latest_sample_utc"], 1129)
        self.assertEqual(candidate["latest_offset_ms"], latest.offset_ms)
        text = ranking_evidence(candidate, latest.data())
        self.assertIn("-7.159", text)
        self.assertIn("-10.945", text)
        self.assertIn("age allowance 1.937", text)

    def test_reference_id_does_not_classify_or_rank_hardware(self):
        ts1, pi = peer("ts1"), peer("pi")
        ts1.latest.reference_id, pi.latest.reference_id = "GPS", "PPS"
        before = recommend([ts1, pi], 100)
        ts1.latest.reference_id, pi.latest.reference_id = "PPS", "GPS"
        after = recommend([ts1, pi], 100)
        self.assertEqual(before["selected"], after["selected"])
        self.assertEqual([c["distance_ms"] for c in before["candidates"]],
                         [c["distance_ms"] for c in after["candidates"]])

    def test_all_sources_have_reasons_and_outlier_is_excluded(self):
        peers = [peer("a", 0), peer("b", 1), peer("c", 2), peer("bad", 500)]
        decision = recommend(peers, 100)
        self.assertIsNotNone(decision["selected"])
        self.assertEqual(len(decision["reasons"]), 4)
        self.assertIn("Outside majority", decision["reasons"]["bad:123"])

    def test_conflict_withholds_recommendation(self):
        decision = recommend([peer("a", 0), peer("b", 500)], 100)
        self.assertIsNone(decision["selected"])

    def test_single_source_is_explicitly_uncorroborated(self):
        decision = recommend([peer("a")], 100)
        self.assertIn("uncorroborated", decision["confidence"])

    def test_stratum_one_does_not_override_quality(self):
        decision = recommend([peer("a", stratum=1, root_dispersion=200), peer("b", stratum=2)], 100)
        self.assertEqual(decision["selected"], "b:123")

    def test_duplicate_endpoint_does_not_get_extra_vote(self):
        decision = recommend([peer("a", address="192.0.2.1"), peer("alias", address="192.0.2.1"),
                              peer("other", 500)], 100)
        self.assertIsNone(decision["selected"])
        self.assertIn("separate vote", decision["reasons"]["alias:123"])

    def test_stale_and_excess_distance(self):
        self.assertIsNone(recommend([peer("a")], 1000)["selected"])
        self.assertIsNone(recommend([peer("a", root_dispersion=2000)], 100)["selected"])

    def test_latest_failure_prevents_stale_success(self):
        p = peer("a")
        p.accept(Sample("a", 123, 0, 101, status="timeout", reason="No reply"))
        self.assertIsNone(recommend([p], 101)["selected"])
        self.assertEqual(p.reach, 2)

    def test_min_delay_filter_and_jitter(self):
        p = peer("a", offset=0, delay=20)
        second = copy.deepcopy(p.latest)
        second.offset_ms, second.delay_ms = 2, 5
        p.accept(second)
        value, _ = p.candidate(100, 1000)
        self.assertEqual(value["offset_ms"], 2)
        self.assertEqual(value["jitter_ms"], 2)


class OperationTests(unittest.TestCase):
    def test_rate_and_access_refusal(self):
        p = Peer("a")
        rate = Sample("a", 123, 0, 0, status="control", kod="RATE", poll=6)
        control_state(p, rate, 0, 64)
        self.assertEqual(p.interval, 128)
        control_state(p, rate, 0, 64)
        self.assertEqual(p.interval, 256)
        rate.kod = "DENY"
        control_state(p, rate, 0, 64)
        self.assertIn("DENY", p.disabled)

    def test_config_rejects_short_history_and_poll_flood(self):
        for name, value in (("retention_days", 6), ("poll_seconds", 1), ("timeout_seconds", float("nan"))):
            cfg = {"servers": [{"host": "a"}], name: value}
            with self.assertRaises(ValueError):
                validate_config(cfg)

    def test_transactional_history_retention_exports_and_restart(self):
        with tempfile.TemporaryDirectory(dir=TEST_ROOT) as folder:
            path = Path(folder) / "history.sqlite3"
            store = Store(path)
            now = time.time()
            samples = [peer("a").latest, peer("b").latest]
            samples[0].utc, samples[1].utc = now - 8 * 86400, now - 6 * 86400
            decision = recommend([peer("a"), peer("b")], 100)
            store.save_cycle(1, samples, decision, {"a:123": {"interval": 128, "disabled": "DENY"}}, now)
            store.event("test", {"details": "recorded"}, now)
            reopened = Store(path)
            self.assertEqual(len(reopened.stats()), 2)
            self.assertEqual(reopened.load_state()["a:123"]["interval"], 128)
            self.assertEqual(reopened.next_cycle(), 2)
            reopened.prune(now)
            self.assertEqual(len(reopened.stats()), 1)
            reopened.export(Path(folder) / "export")
            for name in ("samples", "decisions", "events"):
                self.assertTrue((Path(folder) / "export" / (name + ".csv")).exists())

    def test_every_server_queried_and_restrictions_survive_restart(self):
        with tempfile.TemporaryDirectory(dir=TEST_ROOT) as folder:
            store = Store(Path(folder) / "history.sqlite3")
            cfg = validate_config({"servers": [{"host": "a"}, {"host": "b"}, {"host": "c"}]})
            def fake(host, port, *args):
                value = peer(host).latest
                value.mono, value.utc = time.perf_counter(), time.time()
                return value
            with patch("monitor.query", side_effect=fake) as queried:
                m = Monitor(cfg, store)
                result = m.collect()
                self.assertEqual(queried.call_count, 3)
                self.assertEqual(len(result["servers"]), 3)
                self.assertTrue(all(s["reason"] for s in result["servers"]))
            with patch("monitor.query") as queried:
                self.assertIsNone(Monitor(cfg, store).collect())
                queried.assert_not_called()

    def test_instance_lock(self):
        with tempfile.TemporaryDirectory(dir=TEST_ROOT) as folder:
            path = Path(folder) / "monitor.lock"
            with InstanceLock(path):
                with self.assertRaises(RuntimeError):
                    with InstanceLock(path):
                        pass
            with InstanceLock(path):
                pass


if __name__ == "__main__":
    unittest.main(verbosity=2)
