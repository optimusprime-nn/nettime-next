"""NTP unicast measurement transport. NTS-required requests use nts.py.

Plain NTP accepts only base headers; unsupported extensions/MACs fail closed.
Clock mitigation and the release-gated controller are separate modules.
"""
from __future__ import annotations

import math
import secrets
import socket
import struct
import time
from dataclasses import asdict, dataclass

NTP_EPOCH = 2208988800
ERA = 1 << 32

# Python 3.11's Windows time.time() can be coarser than the precise clock API.
try:
    import ctypes
    _precise = ctypes.windll.kernel32.GetSystemTimePreciseAsFileTime
    _precise.argtypes = [ctypes.POINTER(ctypes.c_ulonglong)]
    _precise.restype = None
except (AttributeError, OSError):
    _precise = None


def utc_now():
    if _precise is None:
        return time.time()
    value = ctypes.c_ulonglong()
    _precise(ctypes.byref(value))
    return (value.value - 116444736000000000) / 10000000


class InvalidReply(ValueError):
    pass


class KissOfDeath(InvalidReply):
    def __init__(self, code: str, poll: int):
        super().__init__(f"Server control response: {code}")
        self.code, self.poll = code, poll


def encode_time(unix: float) -> bytes:
    seconds = unix + NTP_EPOCH
    whole = math.floor(seconds)
    return struct.pack("!II", whole % ERA, int((seconds - whole) * ERA))


def decode_time(raw: bytes, pivot: float) -> float:
    seconds, fraction = struct.unpack("!II", raw)
    base = seconds - NTP_EPOCH + fraction / ERA
    # The local date must be within 68 years of the source, including in 2036.
    return base + round((pivot - base) / ERA) * ERA


@dataclass
class Sample:
    host: str
    port: int
    utc: float
    mono: float
    status: str = "error"
    reason: str = ""
    address: str = ""
    local_address: str = ""
    source_port: int | None = None
    version: int | None = None
    mode: int | None = None
    stratum: int | None = None
    leap: int | None = None
    reference_id: str = ""
    reference_utc: float | None = None
    poll: int | None = None
    precision: int | None = None
    root_delay_ms: float | None = None
    root_dispersion_ms: float | None = None
    offset_ms: float | None = None
    delay_ms: float | None = None
    exchange_ms: float | None = None
    clock_step_ms: float | None = None
    t1: float | None = None
    t2: float | None = None
    t3: float | None = None
    t4: float | None = None
    request_hex: str = ""
    request_transmit_utc: float | None = None
    reply_hex: str = ""
    kod: str = ""
    security_mode: str = "plain_ntp"
    authentication_status: str = "unauthenticated"
    remote_port: int | None = None
    nts_ke_host: str = ""
    nts_ke_port: int | None = None
    tls_version: str = ""
    nts_key_exchange: str = ""
    nts_session_age_seconds: float | None = None
    nts_cookie_count: int | None = None
    certificate_sha256: str = ""

    def data(self):
        return asdict(self)


def parse_reply(raw: bytes, request_stamp: bytes, sample: Sample) -> Sample:
    sample.reply_hex = raw.hex()
    if len(raw) < 48:
        raise InvalidReply(f"Truncated NTP header ({len(raw)} bytes)")
    sample.leap, sample.version, sample.mode = raw[0] >> 6, (raw[0] >> 3) & 7, raw[0] & 7
    sample.stratum = raw[1]
    sample.poll, sample.precision = struct.unpack("!bb", raw[2:4])
    sample.root_delay_ms = struct.unpack("!i", raw[4:8])[0] / 65536 * 1000
    sample.root_dispersion_ms = struct.unpack("!I", raw[8:12])[0] / 65536 * 1000
    sample.reference_id = raw[12:16].hex()
    if sample.stratum <= 1:
        sample.reference_id = raw[12:16].decode("ascii", errors="replace").rstrip("\x00")
    elif sample.version == 3 or ":" not in sample.address:
        sample.reference_id = socket.inet_ntoa(raw[12:16])
    if sample.mode != 4:
        raise InvalidReply(f"Expected server mode 4; received {sample.mode}")
    if sample.version not in (3, 4):
        raise InvalidReply(f"Unsupported NTP version {sample.version}")
    if raw[24:32] != request_stamp:
        raise InvalidReply("Originate timestamp does not match request (bogus/replayed reply)")
    # This observer deliberately does not silently accept unverified MACs.
    if len(raw) != 48:
        raise InvalidReply("Extensions/authentication present; unsupported in this prototype")
    if sample.stratum == 0:
        code = raw[12:16].decode("ascii", errors="replace")
        sample.kod = code
        raise KissOfDeath(code, sample.poll)
    if sample.leap == 3:
        raise InvalidReply("Server reports unsynchronized clock (LI=3)")
    if not 1 <= sample.stratum <= 15:
        raise InvalidReply(f"Invalid/unsynchronized stratum {sample.stratum}")
    if raw[32:40] == bytes(8) or raw[40:48] == bytes(8):
        raise InvalidReply("Missing receive/transmit timestamp")
    sample.t2 = decode_time(raw[32:40], sample.t1)
    sample.t3 = decode_time(raw[40:48], sample.t1)
    if raw[16:24] == bytes(8):
        raise InvalidReply("Missing reference timestamp")
    sample.reference_utc = decode_time(raw[16:24], sample.t3)
    if sample.reference_utc > sample.t3 + 0.001:
        raise InvalidReply("Reference timestamp is later than transmit timestamp")
    if sample.t3 < sample.t2:
        raise InvalidReply("Server transmit time precedes receive time")
    # Detect changes made by NetTime/Windows during this measurement.
    if abs(sample.clock_step_ms or 0) > 5:
        raise InvalidReply("Local clock changed during exchange; sample discarded")
    delay = (sample.t4 - sample.t1) - (sample.t3 - sample.t2)
    tolerance = max(0.001, 2.0 ** max(-32, min(0, sample.precision)))
    if delay < -tolerance:
        raise InvalidReply("Impossible negative round-trip delay")
    sample.delay_ms = max(0.0, delay * 1000)
    sample.offset_ms = ((sample.t2 - sample.t1) + (sample.t3 - sample.t4)) * 500
    sample.status = "valid"
    sample.reason = "Base-header checks passed; unauthenticated measurement"
    return sample


def query(host: str, port: int, timeout: float, poll_seconds: float, cycle: int = 0,
          security_mode: str = "plain_ntp", nts_client=None, ke_port=4460) -> Sample:
    if security_mode == "nts_required":
        from nts import query_nts
        return query_nts(host, port, timeout, poll_seconds, cycle, nts_client, ke_port)
    sample = Sample(host, port, utc_now(), time.perf_counter())
    sample.security_mode = security_mode
    if security_mode != "plain_ntp":
        sample.status = "security_blocked"
        sample.authentication_status = "not_attempted"
        sample.reason = "Unknown security policy; no request sent"
        return sample
    try:
        addresses = socket.getaddrinfo(host, port, type=socket.SOCK_DGRAM)
        # Rotate DNS answers so a dead first address cannot hide other addresses.
        unique = list(dict.fromkeys(addresses))
        family, socktype, proto, _, address = unique[cycle % len(unique)]
        sample.address = address[0]
        with socket.socket(family, socktype, proto) as sock:
            sock.settimeout(timeout)
            # Connected UDP restricts replies to the resolved endpoint. OS picks
            # an ephemeral local port: no collision with an existing NTP service.
            sock.connect(address)
            sample.local_address, sample.source_port = sock.getsockname()[:2]
            packet = bytearray(48)
            packet[0] = (4 << 3) | 3
            packet[2] = max(6, min(17, math.ceil(math.log2(poll_seconds))))
            packet[3] = (-10) & 255  # Conservative millisecond observer precision.
            sample.t1 = utc_now()
            start = time.perf_counter()
            stamp = bytearray(encode_time(sample.t1))
            # Randomize only bits below 60 ns, below the UTC API's resolution.
            # Keep the captured local T1: a request nonce must never replace a
            # measured timestamp used in the four-timestamp calculation.
            stamp[-1:] = secrets.token_bytes(1)
            packet[40:48] = stamp
            sample.request_transmit_utc = decode_time(bytes(stamp), sample.t1)
            sample.request_hex = packet.hex()
            sock.send(packet)
            raw = sock.recv(65535)
            sample.t4 = utc_now()
            end = time.perf_counter()
            sample.mono, sample.utc = end, sample.t4
            sample.exchange_ms = (end - start) * 1000
            sample.clock_step_ms = ((sample.t4 - sample.t1) - (end - start)) * 1000
            return parse_reply(raw, bytes(stamp), sample)
    except KissOfDeath as exc:
        sample.status, sample.reason = "control", str(exc)
    except InvalidReply as exc:
        sample.status, sample.reason = "invalid", str(exc)
    except (socket.timeout, TimeoutError):
        sample.status, sample.reason = "timeout", f"No response within {timeout:g} seconds"
    except OSError as exc:
        sample.status, sample.reason = "network_error", str(exc)
    sample.utc, sample.mono = utc_now(), time.perf_counter()
    return sample
