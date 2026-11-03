//! RFC 8915 transport. Cookies and exporter keys stay in memory and are never serialized.
use crate::{config::Server, protocol::mono};
use aes_siv::{KeyInit, siv::Aes128Siv};
use anyhow::{Result, bail, ensure};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned, pki_types::ServerName};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    net::TcpStream,
    sync::Arc,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;
const UID: u16 = 0x104;
const COOKIE: u16 = 0x204;
const PLACEHOLDER: u16 = 0x304;
const AUTH: u16 = 0x404;
fn word(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}
fn record(kind: u16, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&kind.to_be_bytes());
    out.extend_from_slice(&(body.len() as u16).to_be_bytes());
    out.extend_from_slice(body);
    out
}
fn extension(kind: u16, body: &[u8]) -> Vec<u8> {
    let size = (body.len() + 3) & !3;
    let mut out = record(kind, &vec![0; size]);
    out[2..4].copy_from_slice(&((size + 4) as u16).to_be_bytes());
    out[4..4 + body.len()].copy_from_slice(body);
    out
}
fn fields(data: &[u8], mut pos: usize) -> Result<Vec<(u16, &[u8], usize)>> {
    let mut out = vec![];
    while pos < data.len() {
        ensure!(pos + 4 <= data.len(), "Truncated extension");
        let n = word(&data[pos + 2..]) as usize;
        ensure!(
            n >= 4 && n.is_multiple_of(4) && pos + n <= data.len(),
            "Invalid extension length"
        );
        out.push((word(&data[pos..]), &data[pos + 4..pos + n], pos));
        pos += n;
    }
    Ok(out)
}
/// A key-exchange socket whose reads and writes share one deadline. The socket
/// timeout restarts with every byte received, so a peer (or anyone on the path,
/// before the certificate is checked) trickling bytes could otherwise hold the
/// exchange, and the whole polling round waiting on it, open for hours.
struct Deadline {
    socket: TcpStream,
    until: Instant,
}
/// A socket timeout is the deadline running out; report it as such.
fn expired(e: std::io::Error) -> std::io::Error {
    match e.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "NTS-KE deadline exceeded")
        }
        _ => e,
    }
}
impl Deadline {
    fn remaining(&self) -> std::io::Result<Duration> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(expired(std::io::ErrorKind::TimedOut.into()));
        }
        Ok(left)
    }
}
impl Read for Deadline {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.socket.set_read_timeout(Some(self.remaining()?))?;
        let read = self.socket.read(buf);
        read.map_err(expired)
    }
}
impl Write for Deadline {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.socket.set_write_timeout(Some(self.remaining()?))?;
        let written = self.socket.write(buf);
        written.map_err(expired)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.socket.flush()
    }
}
pub struct Session {
    pub host: String,
    pub port: u16,
    c2s: Zeroizing<Vec<u8>>,
    s2c: Zeroizing<Vec<u8>>,
    pub cookies: VecDeque<Zeroizing<Vec<u8>>>,
    pub established: f64,
    pub fingerprint: String,
}
impl Session {
    pub fn request(&mut self, header: &[u8]) -> Result<(Vec<u8>, Vec<u8>, usize)> {
        let cookie = self
            .cookies
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("NTS cookie pool empty"))?;
        let mut uid = vec![0; 32];
        getrandom::fill(&mut uid).map_err(|e| anyhow::anyhow!("Random source: {e}"))?;
        let mut prefix = header.to_vec();
        prefix.extend(extension(UID, &uid));
        prefix.extend(extension(COOKIE, &cookie));
        let mut count = 0;
        while count < 7 - self.cookies.len() && prefix.len() + cookie.len() + 44 <= 1200 {
            prefix.extend(extension(PLACEHOLDER, &vec![0; cookie.len()]));
            count += 1;
        }
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).map_err(|e| anyhow::anyhow!("Random source: {e}"))?;
        let mut cipher =
            Aes128Siv::new_from_slice(&self.c2s).map_err(|_| anyhow::anyhow!("Invalid NTS key"))?;
        let encrypted = cipher
            .encrypt([prefix.as_slice(), &nonce], &[])
            .map_err(|_| anyhow::anyhow!("NTS encryption failed"))?;
        let mut body = Vec::new();
        body.extend_from_slice(&16u16.to_be_bytes());
        body.extend_from_slice(&(encrypted.len() as u16).to_be_bytes());
        body.extend(nonce);
        body.extend(encrypted);
        prefix.extend(extension(AUTH, &body));
        Ok((prefix, uid, count + 1))
    }
    pub fn verify(&mut self, raw: &[u8], uid: &[u8], limit: usize) -> Result<()> {
        ensure!(
            raw.len() >= 48 && raw[0] & 0x3f == 0x24,
            "NTS requires NTPv4 server reply"
        );
        let all = fields(raw, 48)?;
        let authenticators: Vec<_> = all.iter().filter(|f| f.0 == AUTH).collect();
        ensure!(
            authenticators.len() == 1,
            "Missing/duplicate NTS authenticator; plaintext fallback forbidden"
        );
        let (_, body, pos) = *authenticators[0];
        let before: Vec<_> = all.iter().filter(|f| f.2 < pos).collect();
        let ids: Vec<_> = before.iter().filter(|f| f.0 == UID).collect();
        ensure!(
            ids.len() == 1 && ids[0].1 == uid,
            "NTS unique identifier mismatch"
        );
        ensure!(
            !before.iter().any(|f| [COOKIE, PLACEHOLDER].contains(&f.0)),
            "Unencrypted cookie"
        );
        ensure!(body.len() >= 4, "Truncated NTS authenticator");
        let nl = word(body) as usize;
        let cl = word(&body[2..]) as usize;
        let np = (nl + 3) & !3;
        ensure!(
            nl >= 1 && cl >= 16 && 4 + np + ((cl + 3) & !3) <= body.len(),
            "Invalid NTS nonce/ciphertext length"
        );
        let mut cipher =
            Aes128Siv::new_from_slice(&self.s2c).map_err(|_| anyhow::anyhow!("Invalid NTS key"))?;
        let plain = Zeroizing::new(
            cipher
                .decrypt([&raw[..pos], &body[4..4 + nl]], &body[4 + np..4 + np + cl])
                .map_err(|_| anyhow::anyhow!("NTS authentication failed"))?,
        );
        let decoded = fields(&plain, 0)?;
        ensure!(
            !decoded
                .iter()
                .any(|f| [UID, AUTH, PLACEHOLDER].contains(&f.0)),
            "Invalid encrypted extension"
        );
        let cookies: Vec<_> = decoded.iter().filter(|f| f.0 == COOKIE).collect();
        ensure!(
            !cookies.is_empty()
                && cookies.len() <= limit
                && cookies.iter().all(|f| !f.1.is_empty() && f.1.len() <= 1024),
            "Invalid NTS replenishment cookies"
        );
        for c in cookies {
            if self.cookies.len() == 8 {
                self.cookies.pop_front();
            }
            self.cookies.push_back(Zeroizing::new(c.1.to_vec()));
        }
        Ok(())
    }
}
fn establish(server: &Server, timeout: f64) -> Result<Session> {
    // Name resolution, connection, the TLS handshake and the key-exchange
    // records take at least three round trips; together they get three times
    // the per-request timeout, and no step may wait past that deadline.
    let until = Instant::now() + Duration::from_secs_f64(3. * timeout);
    let left = || until.saturating_duration_since(Instant::now());
    // Try each resolved address in turn; one unreachable address (for example
    // IPv6 without a route) must not block key establishment.
    let mut last = anyhow::anyhow!("No NTS-KE address");
    let mut connected = None;
    for address in crate::protocol::resolve(&server.host, server.nts_ke_port, timeout)? {
        let wait = left().min(Duration::from_secs_f64(timeout));
        if wait.is_zero() {
            last = anyhow::anyhow!("NTS-KE deadline exceeded");
            break;
        }
        match TcpStream::connect_timeout(&address, wait) {
            Ok(socket) => {
                connected = Some((address, socket));
                break;
            }
            Err(e) => last = e.into(),
        }
    }
    let (endpoint, socket) = connected.ok_or(last)?;
    let socket = Deadline { socket, until };
    let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut cfg = ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_root_certificates(roots)
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"ntske/1".to_vec()];
    let conn = ClientConnection::new(Arc::new(cfg), ServerName::try_from(server.host.clone())?)?;
    let mut tls = StreamOwned::new(conn, socket);
    while tls.conn.is_handshaking() {
        tls.conn.complete_io(&mut tls.sock)?;
    }
    ensure!(
        tls.conn.alpn_protocol() == Some(b"ntske/1"),
        "NTS-KE ALPN mismatch"
    );
    let fingerprint = hex::encode(Sha256::digest(
        tls.conn
            .peer_certificates()
            .ok_or_else(|| anyhow::anyhow!("No certificate"))?[0]
            .as_ref(),
    ));
    tls.write_all(
        &[
            record(0x8001, &[0, 0]),
            record(0x8004, &[0, 15]),
            record(0x8000, &[]),
        ]
        .concat(),
    )?;
    tls.flush()?;
    let mut values = HashMap::new();
    let mut cookies = VecDeque::new();
    let mut total = 0;
    loop {
        ensure!(total < 65536, "NTS-KE response too large");
        let mut head = [0; 4];
        tls.read_exact(&mut head)?;
        let tag = word(&head);
        let kind = tag & 0x7fff;
        let size = word(&head[2..]) as usize;
        total += 4 + size;
        ensure!(total <= 65536, "NTS-KE response too large");
        let mut body = vec![0; size];
        tls.read_exact(&mut body)?;
        match kind {
            0 => {
                ensure!(tag == 0x8000 && size == 0, "Invalid end record");
                break;
            }
            2 | 3 => bail!("NTS-KE server error/warning"),
            5 => {
                ensure!(
                    !body.is_empty() && size.is_multiple_of(4) && size <= 1024,
                    "Invalid cookie"
                );
                if cookies.len() < 8 {
                    cookies.push_back(Zeroizing::new(body));
                }
            }
            1 | 4 | 6 | 7 => {
                ensure!(!values.contains_key(&kind), "Duplicate NTS-KE record");
                if kind == 1 {
                    ensure!(tag & 0x8000 != 0, "Next protocol must be critical")
                };
                values.insert(kind, body);
            }
            _ => ensure!(tag & 0x8000 == 0, "Unknown critical NTS-KE record"),
        }
    }
    ensure!(
        values.get(&1).map(Vec::as_slice) == Some(&[0, 0])
            && values.get(&4).map(Vec::as_slice) == Some(&[0, 15])
            && !cookies.is_empty(),
        "NTS negotiation incomplete"
    );
    let host = if let Some(h) = values.get(&6) {
        let h = std::str::from_utf8(h)?;
        ensure!(
            !h.is_empty()
                && h.len() <= 253
                && !h.chars().any(|c| c.is_whitespace() || "/\\%\0".contains(c)),
            "Invalid negotiated host"
        );
        if h.parse::<std::net::IpAddr>().is_ok() {
            h.into()
        } else {
            format!("{}.", h.trim_end_matches('.'))
        }
    } else {
        endpoint.ip().to_string()
    };
    let port = if let Some(p) = values.get(&7) {
        ensure!(p.len() == 2 && word(p) > 0, "Invalid negotiated port");
        word(p)
    } else {
        123
    };
    let key = |direction| -> Result<Zeroizing<Vec<u8>>> {
        let out = vec![0; 32];
        Ok(Zeroizing::new(tls.conn.export_keying_material(
            out,
            b"EXPORTER-network-time-security",
            Some(&[0, 0, 0, 15, direction]),
        )?))
    };
    Ok(Session {
        host,
        port,
        c2s: key(0)?,
        s2c: key(1)?,
        cookies,
        established: mono(),
        fingerprint,
    })
}
/// Longest wait between failed NTS-KE attempts. Longer waits leave an NTS source
/// unused for hours after the network returns.
pub const MAX_BACKOFF: f64 = 3600.;
/// Consecutive failed exchanges after which the session is presumed unusable
/// (for example rotated server keys) and NTS-KE runs again.
pub const MAX_MISSES: u32 = 3;
#[derive(Default)]
pub struct NtsClient {
    pub session: Option<Session>,
    pub failures: u32,
    pub retry_at: f64,
    /// Consecutive failed exchanges on the current session.
    pub misses: u32,
    /// Which resolved NTP address to use. It advances after two consecutive
    /// failed exchanges (one loss keeps the address and its sample filter), so
    /// a server with several addresses is not stuck on an unreachable one.
    /// Plain-NTP sources use it too.
    pub address_cursor: usize,
    pub address_failures: u32,
}
impl NtsClient {
    /// Establishes a session when needed. Only real NTS-KE attempts count as
    /// failures: a poll refused during the backoff must not extend the backoff.
    pub fn prepare(&mut self, s: &Server, timeout: f64) -> Result<()> {
        if self
            .session
            .as_ref()
            .is_none_or(|n| n.cookies.is_empty() || mono() - n.established > 86400.)
        {
            ensure!(mono() >= self.retry_at, "NTS key establishment backoff");
            self.session = None;
            match establish(s, timeout) {
                Ok(session) => {
                    self.session = Some(session);
                    self.misses = 0;
                }
                Err(e) => {
                    self.failed();
                    return Err(e);
                }
            }
        }
        Ok(())
    }
    /// A failed exchange on an established session. Its cookie is already spent,
    /// so later polls use the next one (RFC 8915 section 5.7). A lost packet must
    /// not discard the remaining cookies and force a new key exchange.
    pub fn exchange_failed(&mut self) {
        self.misses += 1;
        if self.misses >= MAX_MISSES {
            self.misses = 0;
            self.failed();
        }
    }
    pub fn exchange_succeeded(&mut self) {
        self.failures = 0;
        self.retry_at = 0.;
        self.misses = 0;
    }
    fn failed(&mut self) {
        self.failures = (self.failures + 1).min(30);
        self.retry_at = mono() + (10. * 1.5_f64.powi(self.failures as i32 - 1)).min(MAX_BACKOFF);
        self.session = None;
    }
}

#[cfg(test)]
impl Session {
    /// A session with fixed keys and placeholder cookies for offline tests.
    pub fn for_test(host: &str, port: u16, cookies: usize) -> Self {
        Session {
            host: host.into(),
            port,
            c2s: Zeroizing::new(vec![1; 32]),
            s2c: Zeroizing::new(vec![2; 32]),
            cookies: (0..cookies).map(|_| Zeroizing::new(vec![7; 64])).collect(),
            established: mono(),
            fingerprint: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aes_siv_matches_python_interoperability_vector() {
        let key: Vec<u8> = (0..32).collect();
        let prefix: Vec<u8> = (0..48).collect();
        let nonce: Vec<u8> = (0..16).collect();
        let mut cipher = Aes128Siv::new_from_slice(&key).unwrap();
        assert_eq!(
            hex::encode(cipher.encrypt([&prefix, &nonce], &[]).unwrap()),
            "c7a580e4e580a2955d9661e0d60dce0b"
        );
    }
    /// A server reply carrying `uid`, then `before` in the clear, then an
    /// authenticator sealing `plain` with `key` (the server-to-client key).
    fn reply(uid: &[u8], before: &[u8], plain: &[u8], key: &[u8]) -> Vec<u8> {
        let mut raw = vec![0; 48];
        raw[0] = 0x24;
        raw.extend(extension(UID, uid));
        raw.extend_from_slice(before);
        let nonce = [9; 16];
        let sealed = Aes128Siv::new_from_slice(key)
            .unwrap()
            .encrypt([raw.as_slice(), &nonce], plain)
            .unwrap();
        let mut body = vec![];
        body.extend_from_slice(&16u16.to_be_bytes());
        body.extend_from_slice(&(sealed.len() as u16).to_be_bytes());
        body.extend_from_slice(&nonce);
        body.extend(sealed);
        raw.extend(extension(AUTH, &body));
        raw
    }
    #[test]
    fn authentic_reply_replenishes_cookies() {
        let mut session = Session::for_test("server.test", 123, 0);
        let uid = [4; 32];
        let cookies = [extension(COOKIE, &[5; 64]), extension(COOKIE, &[6; 64])].concat();
        session
            .verify(&reply(&uid, &[], &cookies, &[2; 32]), &uid, 2)
            .unwrap();
        assert_eq!(session.cookies.len(), 2);
        assert_eq!(*session.cookies[1], vec![6; 64]);
    }
    #[test]
    fn forged_or_malformed_replies_are_rejected_without_taking_cookies() {
        let uid = [4; 32];
        let cookie = extension(COOKIE, &[5; 64]);
        let good = reply(&uid, &[], &cookie, &[2; 32]);
        let mut header_changed = good.clone();
        header_changed[40] ^= 1;
        let mut ciphertext_changed = good.clone();
        *ciphertext_changed.last_mut().unwrap() ^= 1;
        let mut client_mode = good.clone();
        client_mode[0] = 0x23;
        let no_authenticator = good[..48 + 36].to_vec();
        let mut two_authenticators = good.clone();
        two_authenticators.extend_from_slice(&good[48 + 36..]);
        let cases = [
            (header_changed, "NTS authentication failed"),
            (ciphertext_changed, "NTS authentication failed"),
            (
                reply(&uid, &[], &cookie, &[3; 32]),
                "NTS authentication failed",
            ),
            (
                reply(&[8; 32], &[], &cookie, &[2; 32]),
                "unique identifier mismatch",
            ),
            (
                reply(&uid, &cookie, &cookie, &[2; 32]),
                "Unencrypted cookie",
            ),
            (
                reply(&uid, &extension(PLACEHOLDER, &[0; 64]), &cookie, &[2; 32]),
                "Unencrypted cookie",
            ),
            (client_mode, "NTPv4 server reply"),
            (no_authenticator, "plaintext fallback forbidden"),
            (two_authenticators, "plaintext fallback forbidden"),
            (
                reply(
                    &uid,
                    &[],
                    &[cookie.clone(), extension(UID, &uid)].concat(),
                    &[2; 32],
                ),
                "Invalid encrypted extension",
            ),
            (reply(&uid, &[], &[], &[2; 32]), "replenishment cookies"),
            (
                reply(&uid, &[], &cookie.repeat(3), &[2; 32]),
                "replenishment cookies",
            ),
        ];
        for (raw, expected) in cases {
            let mut session = Session::for_test("server.test", 123, 1);
            let error = session.verify(&raw, &uid, 2).unwrap_err();
            assert!(
                format!("{error:#}").contains(expected),
                "{expected}: {error:#}"
            );
            assert_eq!(session.cookies.len(), 1, "{expected}");
            assert_eq!(*session.cookies[0], vec![7; 64], "{expected}");
        }
    }
    #[test]
    fn rejects_truncated_and_misaligned_extensions() {
        assert!(fields(&[1, 4, 0, 8, 0], 0).is_err());
        assert!(fields(&[1, 4, 0, 5, 0], 0).is_err());
    }
    #[test]
    fn trickling_key_exchange_peer_cannot_outlast_the_deadline() {
        // Announces a 16 KiB TLS record, then sends one byte of it well inside
        // the read timeout, which kept the exchange open for hours.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let _ = s.write_all(&[0x16, 0x03, 0x03, 0x40, 0x00]);
            for _ in 0..0x4000 {
                std::thread::sleep(Duration::from_millis(50));
                if s.write_all(&[0]).is_err() {
                    break;
                }
            }
        });
        let server = Server {
            host: "127.0.0.1".into(),
            nts_ke_port: port,
            security: "nts_required".into(),
            ..Default::default()
        };
        let started = Instant::now();
        let error = establish(&server, 0.25).err().expect("exchange must fail");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        assert!(format!("{error:#}").contains("deadline"), "{error:#}");
    }
}
