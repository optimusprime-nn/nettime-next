from __future__ import annotations

import json
import math
import queue
import threading
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

from protocol import Sample, query, utc_now
from selection import POLICY, Peer, recommend
from storage import Store

BASE = Path(__file__).resolve().parent


def load_config(path=BASE / "settings.json"):
    cfg = json.loads(Path(path).read_text(encoding="utf-8-sig"))
    return validate_config(cfg)


def validate_config(cfg):
    if not isinstance(cfg.get("servers"), list) or not 1 <= len(cfg["servers"]) <= 32:
        raise ValueError("Configure between 1 and 32 servers")
    keys = set()
    for server in cfg["servers"]:
        host, port = server.get("host", ""), server.get("port", 123)
        if not isinstance(host, str) or not host.strip() or host != host.strip():
            raise ValueError("Each server needs a hostname without surrounding whitespace")
        if not isinstance(port, int) or isinstance(port, bool) or not 1 <= port <= 65535:
            raise ValueError("Server ports must be integers between 1 and 65535")
        key = host.lower(), port
        if key in keys:
            raise ValueError(f"Duplicate server: {host}:{port}")
        keys.add(key)
        server["port"] = port
        security = server.setdefault("security", "plain_ntp")
        if security not in ("plain_ntp", "nts_required"):
            raise ValueError("Server security must be plain_ntp or nts_required")
    for name, default, lower, upper in (
            ("poll_seconds", 64, 64, 131072),
            ("timeout_seconds", 3, 0.1, 30),
            ("retention_days", 7, 7, 3650),
            ("max_distance_ms", 1000, 1, 10000)):
        value = cfg.setdefault(name, default)
        if isinstance(value, bool) or not isinstance(value, (float, int)) or not math.isfinite(value) or not lower <= value <= upper:
            raise ValueError(f"{name} must be between {lower} and {upper}")
    return cfg


def control_state(peer, sample, now, baseline):
    """Persist server-imposed restrictions; manual refresh never bypasses them."""
    if sample.status == "control":
        if sample.kod in ("DENY", "RSTR"):
            peer.disabled = f"Server {sample.kod}: association stopped; operator review required"
        elif sample.kod == "RATE":
            requested = 2 ** max(6, min(17, sample.poll))
            # Repeated RATE responses must continue slowing requests.
            peer.interval = max(peer.interval * 2, requested, baseline)
    peer.next_due = now + peer.interval


class Monitor:
    def __init__(self, cfg, store, messages=None):
        self.cfg, self.store = cfg, store
        self.messages = messages if messages is not None else queue.Queue()
        self.stop = threading.Event()
        self.peers = [Peer(s["host"], s["port"], interval=cfg["poll_seconds"]) for s in cfg["servers"]]
        self.cycle = store.next_cycle()
        self.previous = None
        self.last_wall, self.last_mono = utc_now(), time.perf_counter()
        self.next_prune = self.last_mono + 3600
        for peer in self.peers:
            state = store.load_state().get(peer.key, {})
            peer.interval = max(cfg["poll_seconds"], state.get("interval", 0))
            peer.disabled = state.get("disabled", "")
            # Honor minimum time between requests even across process restarts.
            peer.next_due = self.last_mono + max(0, state.get("next_utc", 0) - self.last_wall)

    def detect_clock_change(self):
        wall, mono = utc_now(), time.perf_counter()
        step = (wall - self.last_wall) - (mono - self.last_mono)
        if abs(step) > 0.05:
            self.store.event("local_clock_discontinuity", {"step_ms": step * 1000,
                "action": "Cleared sample filters; cause unknown (NetTime/Windows/manual change possible)"})
            for peer in self.peers:
                peer.samples.clear()
            # Do not delete history following an unexpected wall-clock jump.
            self.next_prune = mono + 86400
        if mono - self.last_mono > 3 * self.cfg["poll_seconds"]:
            self.store.event("sampling_gap", {"seconds": mono - self.last_mono})
        self.last_wall, self.last_mono = wall, mono

    def collect(self):
        self.detect_clock_change()
        now = time.perf_counter()
        due = [p for p in self.peers if not p.disabled and p.next_due <= now]
        if not due:
            return None
        samples = []
        batch_wall, batch_mono = utc_now(), time.perf_counter()
        with ThreadPoolExecutor(max_workers=min(8, len(due))) as pool:
            jobs = {pool.submit(query, p.host, p.port, self.cfg["timeout_seconds"],
                                p.interval, self.cycle,
                                next(s["security"] for s in self.cfg["servers"]
                                     if s["host"] == p.host and s["port"] == p.port)): p for p in due}
            for future in as_completed(jobs):
                peer = jobs[future]
                sample = future.result()
                if abs(sample.clock_step_ms or 0) > 5:
                    for other in self.peers:
                        other.samples.clear()
                    self.store.event("exchange_clock_discontinuity", sample.data())
                peer.accept(sample)
                control_state(peer, sample, time.perf_counter(), self.cfg["poll_seconds"])
                samples.append(sample)
        now, wall = time.perf_counter(), utc_now()
        batch_step = (wall - batch_wall) - (now - batch_mono)
        if abs(batch_step) > 0.005 or any(abs(s.clock_step_ms or 0) > 5 for s in samples):
            self.store.event("cycle_clock_discontinuity", {"step_ms": batch_step * 1000,
                "action": "Discarded all valid measurements from affected cycle; cleared filters"})
            for peer in self.peers:
                peer.samples.clear()
            for s in samples:
                if s.status == "valid":
                    s.status = "invalid"
                    s.reason = "Local clock discontinuity during cycle; measurements discarded"
                    s.offset_ms = s.delay_ms = None
                    peer = next(p for p in self.peers if p.host == s.host and p.port == s.port)
                    peer.reach &= 254
        decision = recommend(self.peers, now, self.cfg["max_distance_ms"])
        # Include every configured entry in every decision, including not-due
        # or demobilized entries, with the exact next-check time/reason.
        decision["servers"] = []
        for peer in self.peers:
            peer.recommendation = decision["reasons"][peer.key]
            decision["servers"].append(dict(peer=peer.key, reach_octal=f"{peer.reach:03o}",
                next_check_utc=None if peer.disabled else wall + max(0, peer.next_due - now),
                interval_seconds=peer.interval, disabled=peer.disabled,
                latest=peer.latest.data() if peer.latest else None,
                reason=peer.recommendation))
        decision["previous_selected"] = self.previous
        decision["selection_changed"] = decision["selected"] != self.previous
        decision["utc"] = wall
        state = {p.key: dict(interval=p.interval, disabled=p.disabled,
                 next_utc=wall + max(0, p.next_due - now)) for p in self.peers}
        self.store.save_cycle(self.cycle, samples, decision, state, wall)
        self.previous = decision["selected"]
        self.cycle += 1
        self.messages.put(("cycle", decision))
        return decision

    def run(self):
        try:
            self.store.event("monitor_started", {"config": self.cfg, "policy": POLICY,
                "mode": "read-only; existing NetTime controls PC clock",
                "authentication": "none", "source_diversity": "not verified"})
            while not self.stop.is_set():
                self.collect()
                self.detect_clock_change()
                if time.perf_counter() >= self.next_prune:
                    self.store.prune()
                    self.next_prune = time.perf_counter() + 3600
                self.stop.wait(0.5)
        except Exception as exc:
            self.messages.put(("error", f"Monitoring stopped: {exc}"))
        finally:
            try:
                self.store.event("monitor_stopped", {"reason": "stop requested" if self.stop.is_set() else "error"})
            except Exception:
                pass
            self.messages.put(("stopped", None))


class InstanceLock:
    """OS releases the lock on crash; prevents duplicate polling/data writers."""
    def __init__(self, path):
        self.path, self.file = Path(path), None

    def __enter__(self):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.file = self.path.open("a+b")
        self.file.seek(0)
        self.file.write(b"0")
        self.file.flush()
        self.file.seek(0)
        try:
            if __import__("os").name == "nt":
                import msvcrt
                msvcrt.locking(self.file.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(self.file, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            self.file.close()
            self.file = None
            raise RuntimeError("NetTime Next is already running with this data directory")
        return self

    def __exit__(self, *args):
        if self.file:
            self.file.close()
