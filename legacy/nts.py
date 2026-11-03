"""RFC 8915 client: TLS 1.3 NTS-KE and AES-SIV authenticated NTP.

Session secrets are memory-only. This module never logs keys, cookies or complete
NTS datagrams. Each configured association owns its own client instance.
"""
from collections import deque
from dataclasses import dataclass, field
import ipaddress
import secrets
import select
import socket
import struct
import time

from cryptography.hazmat.primitives.ciphers.aead import AESSIV
from cryptography.exceptions import InvalidTag

UID, COOKIE, PLACEHOLDER, AUTH = 0x0104, 0x0204, 0x0304, 0x0404


class NTSError(ValueError):
    pass


def record(kind, body=b"", critical=True):
    return struct.pack("!HH", kind | (0x8000 if critical else 0), len(body)) + body


def extension(kind, body):
    body += bytes((-len(body)) % 4)
    return struct.pack("!HH", kind, len(body) + 4) + body


def extensions(data, start=0):
    while start < len(data):
        if len(data) - start < 4:
            raise NTSError("Truncated extension header")
        kind, length = struct.unpack_from("!HH", data, start)
        if length < 4 or length % 4 or start + length > len(data):
            raise NTSError("Invalid extension length")
        yield kind, data[start + 4:start + length], start
        start += length


def parse_ke(records, default_host):
    values, cookies, offset, ended = {}, [], 0, False
    while offset < len(records):
        if offset + 4 > len(records):
            raise NTSError("Truncated NTS-KE record")
        tag, size = struct.unpack_from("!HH", records, offset)
        kind, critical = tag & 0x7fff, bool(tag & 0x8000)
        offset += 4
        if offset + size > len(records):
            raise NTSError("Truncated NTS-KE body")
        body = records[offset:offset + size]
        offset += size
        if kind == 0:
            if not critical or size or offset != len(records):
                raise NTSError("Invalid NTS-KE end record")
            ended = True
            break
        if kind in (2, 3):
            raise NTSError("NTS-KE server returned an error or warning")
        if kind == 5:
            if not body or len(body) % 4 or len(body) > 1024:
                raise NTSError("Invalid NTS cookie size")
            cookies.append(body)
        elif kind in (1, 4, 6, 7):
            if kind in values:
                raise NTSError("Duplicate NTS-KE negotiation record")
            if kind == 1 and not critical:
                raise NTSError("Next protocol record must be critical")
            values[kind] = body
        elif critical:
            raise NTSError("Unknown critical NTS-KE record")
    if not ended or values.get(1) != b"\x00\x00" or values.get(4) != b"\x00\x0f" or not cookies:
        raise NTSError("NTS-KE did not negotiate NTPv4, AES-SIV and cookies")
    try:
        host = values[6].decode("ascii") if 6 in values else default_host
        if not host or len(host) > 253 or any(c.isspace() or c in '/\\%\x00' for c in host):
            raise ValueError()
        try:
            ipaddress.ip_address(host)
        except ValueError:
            # RFC 8915 requires a negotiated DNS name to be treated as FQDN.
            if 6 in values:
                host = host.rstrip('.') + '.'
        if 7 in values and len(values[7]) != 2:
            raise ValueError()
        port = struct.unpack("!H", values[7])[0] if 7 in values else 123
        if port == 0:
            raise ValueError()
    except (UnicodeError, ValueError, struct.error):
        raise NTSError("Invalid negotiated NTP endpoint") from None
    return host, port, deque(cookies[:8], maxlen=8)


def authenticate(prefix, key, plaintext=b"", nonce=None):
    nonce = secrets.token_bytes(16) if nonce is None else nonce
    # RFC 5297: nonce is the final associated-data component.
    ciphertext = AESSIV(key).encrypt(plaintext, [prefix, nonce])
    body = struct.pack("!HH", len(nonce), len(ciphertext))
    body += nonce + bytes((-len(nonce)) % 4)
    body += ciphertext + bytes((-len(ciphertext)) % 4)
    return prefix + extension(AUTH, body)


@dataclass(repr=False)
class Session:
    host: str
    port: int
    c2s: bytes = field(repr=False)
    s2c: bytes = field(repr=False)
    cookies: deque = field(repr=False)
    established: float = 0
    tls_version: str = "TLSv1.3"
    certificate_sha256: str = ""

    def request(self, header):
        if not self.cookies:
            raise NTSError("No unused NTS cookies")
        cookie = self.cookies.popleft()  # Consume even when an exchange fails.
        uid = secrets.token_bytes(32)
        prefix = bytes(header) + extension(UID, uid) + extension(COOKIE, cookie)
        count = 0
        # Keep the UDP payload below a conservative fragmentation boundary.
        while count < 7 - len(self.cookies) and len(prefix) + len(cookie) + 4 + 40 <= 1200:
            prefix += extension(PLACEHOLDER, bytes(len(cookie)))
            count += 1
        return authenticate(prefix, self.c2s), uid, count + 1

    def verify(self, raw, uid, max_cookies):
        if len(raw) < 48 or (raw[0] >> 3) & 7 != 4 or raw[0] & 7 != 4:
            raise NTSError("NTS requires an NTPv4 server reply")
        fields = list(extensions(raw, 48))
        authenticators = [f for f in fields if f[0] == AUTH]
        if len(authenticators) != 1:
            # NTSN can request a fresh handshake, but never counts as time.
            if raw[1] == 0 and raw[12:16] == b"NTSN" and any(k == UID and b == uid for k, b, _ in fields):
                raise NTSError("NTS NAK; fresh key establishment needed")
            raise NTSError("Missing or duplicate NTS authenticator; no plaintext fallback")
        _, body, position = authenticators[0]
        before = [f for f in fields if f[2] < position]
        ids = [b for k, b, _ in before if k == UID]
        if ids != [uid] or any(k in (COOKIE, PLACEHOLDER) for k, _, _ in before):
            raise NTSError("NTS identifier mismatch or unencrypted cookie")
        if len(body) < 4:
            raise NTSError("Truncated NTS authenticator")
        nlen, clen = struct.unpack_from("!HH", body)
        padded_nonce = (nlen + 3) & ~3
        needed = 4 + padded_nonce + ((clen + 3) & ~3)
        if nlen < 1 or clen < 16 or needed > len(body):
            raise NTSError("Invalid NTS nonce/ciphertext lengths")
        nonce = body[4:4 + nlen]
        ciphertext = body[4 + padded_nonce:4 + padded_nonce + clen]
        try:
            plaintext = AESSIV(self.s2c).decrypt(ciphertext, [raw[:position], nonce])
        except InvalidTag:
            raise NTSError("NTS authentication failed") from None
        decrypted = list(extensions(plaintext))
        if any(k in (UID, AUTH, PLACEHOLDER) for k, _, _ in decrypted):
            raise NTSError("Invalid encrypted NTS extension")
        cookies = [b for k, b, _ in decrypted if k == COOKIE]
        if not 1 <= len(cookies) <= max_cookies or any(not c or len(c) > 1024 for c in cookies):
            raise NTSError("Invalid replenishment cookies")
        # Ignore extensions after AUTH: they are unauthenticated, RFC 8915 §5.7.
        return cookies


def establish(host, timeout, ke_port=4460, ca_file=None):
    from OpenSSL import SSL
    from service_identity.pyopenssl import verify_hostname, verify_ip_address
    import certifi
    context = SSL.Context(SSL.TLS_CLIENT_METHOD)
    context.set_min_proto_version(SSL.TLS1_3_VERSION)
    context.set_verify(SSL.VERIFY_PEER, lambda conn, cert, errno, depth, ok: ok)
    context.load_verify_locations(ca_file or certifi.where())
    context.set_alpn_protos([b"ntske/1"])
    hostname = host.encode("idna").decode("ascii")
    deadline = time.monotonic() + timeout
    with socket.create_connection((hostname, ke_port), timeout=timeout) as sock:
        sock.setblocking(False)
        conn = SSL.Connection(context, sock)
        conn.set_connect_state()
        try:
            ipaddress.ip_address(hostname)
        except ValueError:
            conn.set_tlsext_host_name(hostname.encode("ascii"))

        def call(function, *args):
            while True:
                try:
                    return function(*args)
                except (SSL.WantReadError, SSL.WantWriteError) as exc:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        raise TimeoutError("NTS-KE timed out")
                    readable = isinstance(exc, SSL.WantReadError)
                    select.select([sock] if readable else [], [] if readable else [sock], [], remaining)
        try:
            call(conn.do_handshake)
            if conn.get_alpn_proto_negotiated() != b"ntske/1":
                raise NTSError("Server did not negotiate ntske/1")
            try:
                ipaddress.ip_address(hostname)
            except ValueError:
                verify_hostname(conn, hostname)
            else:
                verify_ip_address(conn, hostname)
            request = record(1, b"\x00\x00") + record(4, b"\x00\x0f") + record(0)
            sent = 0
            while sent < len(request):
                sent += call(conn.send, request[sent:])
            response = bytearray()
            while len(response) <= 65536:
                head = bytearray()
                while len(head) < 4:
                    part = call(conn.recv, 4 - len(head))
                    if not part:
                        raise NTSError("NTS-KE closed before end of message")
                    head.extend(part)
                kind, size = struct.unpack("!HH", head)
                if len(response) + 4 + size > 65536:
                    raise NTSError("NTS-KE response exceeds size limit")
                response.extend(head)
                remaining = size
                while remaining:
                    part = call(conn.recv, remaining)
                    if not part:
                        raise NTSError("Truncated NTS-KE response")
                    response.extend(part)
                    remaining -= len(part)
                if kind & 0x7fff == 0:
                    break
            endpoint, port, cookies = parse_ke(bytes(response), sock.getpeername()[0])
            label = b"EXPORTER-network-time-security"
            keys = [conn.export_keying_material(label, 32, struct.pack("!HHB", 0, 15, direction)) for direction in (0, 1)]
            from cryptography.hazmat.primitives import hashes
            fingerprint = conn.get_peer_certificate(as_cryptography=True).fingerprint(hashes.SHA256()).hex()
            return Session(endpoint, port, *keys, cookies, time.monotonic(), conn.get_protocol_version_name(), fingerprint)
        finally:
            try:
                conn.shutdown()
            except SSL.Error:
                pass


class Client:
    def __init__(self):
        self.session = None
        self.failures = 0
        self.retry_at = 0

    def failed(self, now):
        self.failures = min(30, self.failures + 1)
        self.retry_at = now + min(432000, 10 * 1.5 ** (self.failures - 1))
        self.session = None

    def prepare(self, host, timeout, ke_port=4460):
        now = time.monotonic()
        if now < self.retry_at:
            raise NTSError(f"NTS-KE retry backoff: {self.retry_at - now:.0f}s remaining")
        if self.session is None or not self.session.cookies or now - self.session.established > 86400:
            try:
                self.session = establish(host, timeout, ke_port)
            except Exception:
                self.failed(now)
                raise
        return self.session

    def succeeded(self):
        # Reset only after KE *and* an authenticated NTP volley succeed.
        self.failures, self.retry_at = 0, 0


def query_nts(host, port, timeout, poll_seconds, cycle, client=None, ke_port=4460):
    from protocol import Sample, utc_now, encode_time, decode_time, parse_reply, KissOfDeath, InvalidReply
    import math
    client = client or Client()
    sample = Sample(host, port, utc_now(), time.perf_counter(), security_mode="nts_required",
                    authentication_status="failed", nts_ke_host=host, nts_ke_port=ke_port)
    try:
        previous_session = client.session
        session = client.prepare(host, timeout, ke_port)
        sample.nts_key_exchange = 'reused' if session is previous_session else 'established'
        sample.nts_session_age_seconds = max(0, time.monotonic() - session.established)
        sample.certificate_sha256 = session.certificate_sha256
        addresses = list(dict.fromkeys(socket.getaddrinfo(session.host, session.port, type=socket.SOCK_DGRAM)))
        family, socktype, proto, _, address = addresses[cycle % len(addresses)]
        sample.address = address[0]
        sample.remote_port = session.port
        sample.nts_ke_host, sample.nts_ke_port = host, ke_port
        sample.tls_version = session.tls_version
        with socket.socket(family, socktype, proto) as sock:
            sock.settimeout(timeout)
            sock.connect(address)
            sample.local_address, sample.source_port = sock.getsockname()[:2]
            header = bytearray(48)
            header[0], header[2], header[3] = 35, max(6, min(17, math.ceil(math.log2(poll_seconds)))), 246
            sample.t1, start = utc_now(), time.perf_counter()
            stamp = bytearray(encode_time(sample.t1))
            stamp[-1:] = secrets.token_bytes(1)
            header[40:48] = stamp
            packet, uid, max_cookies = session.request(header)
            sample.request_transmit_utc = decode_time(bytes(stamp), sample.t1)
            sample.request_hex = bytes(header).hex()  # No cookies/keys in evidence.
            sock.send(packet)
            raw = sock.recv(65535)
            sample.t4, end = utc_now(), time.perf_counter()
            sample.utc, sample.mono = sample.t4, end
            sample.exchange_ms = (end - start) * 1000
            sample.clock_step_ms = ((sample.t4 - sample.t1) - (end - start)) * 1000
            sample.reply_hex = raw[:48].hex()
            cookies = session.verify(raw, uid, max_cookies)
            sample.authentication_status = "authenticated"
            # The base-header parser sees only the header *after* AEAD verification.
            result = parse_reply(raw[:48], bytes(stamp), sample)
            session.cookies.extend(cookies)
            sample.nts_cookie_count = len(session.cookies)
            client.succeeded()
            result.reason = "NTS authenticated; NTP header and timestamp checks passed"
            return result
    except KissOfDeath as exc:
        sample.status, sample.reason = "control", str(exc)
    except InvalidReply as exc:
        sample.status, sample.reason = "invalid", str(exc)
    except Exception as exc:
        # Security/network/library failures never cause an unauthenticated retry.
        sample.status = "security_blocked"
        if not sample.nts_key_exchange:
            sample.nts_key_exchange = 'failed_or_backoff'
        sample.reason = f"NTS failed ({type(exc).__name__}): {exc}; no plaintext fallback"
        if client.session is not None:
            client.failed(time.monotonic())
    sample.utc, sample.mono = utc_now(), time.perf_counter()
    return sample


def diagnostics():
    from OpenSSL import SSL
    from service_identity.pyopenssl import verify_hostname
    import certifi
    from pathlib import Path
    # Exercise the crypto used in the frozen program; never contact a server.
    key, nonce, ad = bytes(range(32)), bytes(range(16)), b'NTS capability check'
    ciphertext = AESSIV(key).encrypt(b'fixture', [ad, nonce])
    assert AESSIV(key).decrypt(ciphertext, [ad, nonce]) == b'fixture'
    return dict(tls13=bool(SSL.TLS1_3_VERSION), aes_siv=True,
                ca_bundle_exists=Path(certifi.where()).is_file(),
                certificate_identity_verification=callable(verify_hostname),
                plaintext_fallback=False)
