from collections import deque
import datetime as dt
import socket
import ssl
import struct
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest.mock import patch

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import NameOID

from nts import AUTH, COOKIE, PLACEHOLDER, UID, Client, NTSError, Session, authenticate, establish, extension, extensions, parse_ke, query_nts, record
from protocol import encode_time


def session():
    return Session('127.0.0.1', 123, bytes(range(32)), bytes(range(32, 64)), deque([b'C' * 32], maxlen=8))


class NTSPacketTests(unittest.TestCase):
    def test_negotiation_and_reject_duplicate_unknown_critical_or_missing_records(self):
        valid = record(1, b'\x00\x00') + record(4, b'\x00\x0f') + record(5, b'C' * 32, False)
        self.assertEqual(parse_ke(valid + record(0), '192.0.2.1')[:2], ('192.0.2.1', 123))
        endpoint = valid + record(6, b'time.example', False) + record(7, struct.pack('!H', 456), False) + record(0)
        self.assertEqual(parse_ke(endpoint, '192.0.2.1')[:2], ('time.example.', 456))
        for bad in (valid + record(1, b'\0\0') + record(0), valid + record(99) + record(0), valid,
                    valid + record(0, critical=False), record(1, b'\0\0') + record(0),
                    valid + record(7, b'bad') + record(0), valid + record(2, b'\0\1') + record(0)):
            with self.assertRaises(NTSError):
                parse_ke(bad, '192.0.2.1')

    def test_cookie_consumed_uid_strong_and_request_size_bounded(self):
        s = session()
        packet, uid, count = s.request(bytes(48))
        self.assertEqual(len(uid), 32)
        self.assertEqual(len(s.cookies), 0)
        self.assertLessEqual(len(packet), 1200)
        self.assertEqual(count, 8)
        fields = list(extensions(packet, 48))
        self.assertEqual(sum(k == COOKIE for k, _, _ in fields), 1)
        self.assertEqual(sum(k == PLACEHOLDER for k, _, _ in fields), 7)
        with self.assertRaises(NTSError):
            s.request(bytes(48))

    def response(self, s, uid):
        head = bytes([36]) + bytes(47)
        return authenticate(head + extension(UID, uid), s.s2c, extension(COOKIE, b'N' * 32))

    def test_authentication_tamper_replay_and_wrong_key_fail_closed(self):
        s, uid = session(), b'U' * 32
        reply = self.response(s, uid)
        self.assertEqual(s.verify(reply, uid, 1), [b'N' * 32])
        # Every header bit, identifier, and ciphertext is covered by authentication.
        for position in (1, 12, 44, 60, len(reply) - 1):
            corrupt = bytearray(reply)
            corrupt[position] ^= 1
            with self.assertRaises(NTSError):
                s.verify(bytes(corrupt), uid, 1)
        with self.assertRaises(NTSError):
            s.verify(reply, b'V' * 32, 1)
        with self.assertRaises(NTSError):
            s.verify(authenticate(bytes([36]) + bytes(47) + extension(UID, uid), s.c2s, extension(COOKIE, b'N' * 32)), uid, 1)

    def test_unauthenticated_trailing_fields_are_discarded_and_cookies_must_be_encrypted(self):
        s, uid = session(), b'U' * 32
        reply = self.response(s, uid) + extension(COOKIE, b'ATTACKER' * 4)
        self.assertEqual(s.verify(reply, uid, 1), [b'N' * 32])
        bad = authenticate(bytes([36]) + bytes(47) + extension(UID, uid) + extension(COOKIE, b'bad!'), s.s2c, extension(COOKIE, b'good'))
        with self.assertRaisesRegex(NTSError, 'unencrypted'):
            s.verify(bad, uid, 1)

    def test_bad_lengths_duplicate_authenticator_and_plain_reply_are_rejected(self):
        s, uid = session(), b'U' * 32
        reply = self.response(s, uid)
        for bad in (reply[:48], reply[:-1], reply + extension(AUTH, bytes(40)),
                    reply[:48] + b'\x01\x04\x00\x03'):
            with self.assertRaises(NTSError):
                s.verify(bad, uid, 1)

    def test_backoff_only_resets_after_authenticated_volley(self):
        c = Client()
        c.failed(time.monotonic())
        with patch('nts.establish') as ke:
            with self.assertRaisesRegex(NTSError, 'backoff'):
                c.prepare('example', 1)
            ke.assert_not_called()
        c.retry_at = 0
        with patch('nts.establish', return_value=session()):
            c.prepare('example', 1)
        self.assertEqual(c.failures, 1)
        c.succeeded()
        self.assertEqual(c.failures, 0)

    def test_end_to_end_local_authenticated_udp_renews_without_leaking_secrets(self):
        s = session()
        listener = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        listener.bind(('127.0.0.1', 0))
        listener.settimeout(3)
        s.port = listener.getsockname()[1]
        failures = []

        def respond():
            try:
                request, remote = listener.recvfrom(2048)
                fields = list(extensions(request, 48))
                uid = next(b for k, b, _ in fields if k == UID)
                raw = bytearray(48)
                raw[0], raw[1], raw[2], raw[3] = 36, 2, 6, 236
                raw[16:24] = encode_time(time.time() - 10)
                raw[24:32] = request[40:48]
                raw[32:40] = encode_time(time.time())
                raw[40:48] = encode_time(time.time())
                reply = authenticate(bytes(raw) + extension(UID, uid), s.s2c, extension(COOKIE, b'NEW_SECRET_COOKIE' * 4))
                listener.sendto(reply, remote)
            except Exception as exc:
                failures.append(exc)
            finally:
                listener.close()
        thread = threading.Thread(target=respond)
        thread.start()
        client = Client()
        client.session = s
        s.established = time.monotonic()
        result = query_nts('fixture', 123, 2, 64, 0, client)
        thread.join()
        self.assertFalse(failures, failures)
        self.assertEqual(result.status, 'valid', result.reason)
        self.assertEqual(result.authentication_status, 'authenticated')
        self.assertEqual(len(result.request_hex), 96)
        self.assertEqual(len(result.reply_hex), 96)
        self.assertNotIn(b'NEW_SECRET_COOKIE'.hex(), str(result.data()))
        self.assertEqual(list(s.cookies), [b'NEW_SECRET_COOKIE' * 4])


class NTSKeyExchangeTests(unittest.TestCase):
    """Use Python's separate TLS server against the OpenSSL client."""
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.folder = Path(self.temp.name)
        key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
        name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, 'localhost')])
        now = dt.datetime.now(dt.timezone.utc)
        cert = (x509.CertificateBuilder().subject_name(name).issuer_name(name).public_key(key.public_key())
                .serial_number(x509.random_serial_number()).not_valid_before(now - dt.timedelta(days=1))
                .not_valid_after(now + dt.timedelta(days=1)).add_extension(x509.BasicConstraints(ca=True, path_length=None), critical=True)
                .add_extension(x509.SubjectAlternativeName([x509.DNSName('localhost')]), critical=False).sign(key, hashes.SHA256()))
        self.ca = self.folder / 'ca.pem'
        self.ca.write_bytes(cert.public_bytes(serialization.Encoding.PEM))
        self.key = self.folder / 'key.pem'
        self.key.write_bytes(key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))

    def tearDown(self):
        self.temp.cleanup()

    def connect(self, host, port, ca):
        # Resolve the fixture deterministically to its IPv4 listener, while
        # retaining the requested identity for actual certificate validation.
        connect = socket.create_connection
        with patch('nts.socket.create_connection', side_effect=lambda address, timeout: connect(('127.0.0.1', port), timeout)):
            return establish(host, 3, port, ca)

    def server(self, alpn='ntske/1'):
        listener = socket.socket()
        listener.bind(('127.0.0.1', 0))
        listener.listen(1)
        listener.settimeout(3)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.minimum_version = ssl.TLSVersion.TLSv1_3
        context.load_cert_chain(self.ca, self.key)
        context.set_alpn_protocols([alpn])

        def respond():
            try:
                sock, _ = listener.accept()
                with sock, context.wrap_socket(sock, server_side=True) as connection:
                    connection.settimeout(2)
                    request = connection.recv(1024)
                    if request:
                        connection.sendall(record(1, b'\0\0') + record(4, b'\0\x0f') + record(5, b'C' * 32, False) + record(0))
            except (OSError, ssl.SSLError):
                pass  # Expected when the client rejects the certificate/ALPN.
            finally:
                listener.close()
        thread = threading.Thread(target=respond)
        thread.start()
        return listener.getsockname()[1], thread

    def test_tls13_certificate_and_key_export(self):
        port, thread = self.server()
        try:
            s = self.connect('localhost', port, str(self.ca))
            self.assertEqual(s.tls_version, 'TLSv1.3')
            self.assertEqual(len(s.c2s), 32)
            self.assertNotEqual(s.c2s, s.s2c)
        finally:
            thread.join()

    def test_untrusted_certificate_wrong_hostname_and_alpn_are_rejected(self):
        for host, ca, alpn in [('localhost', None, 'ntske/1'), ('127.0.0.1', str(self.ca), 'ntske/1'), ('localhost', str(self.ca), 'wrong')]:
            port, thread = self.server(alpn)
            try:
                with self.assertRaises(Exception):
                    self.connect(host, port, ca)
            finally:
                thread.join()
