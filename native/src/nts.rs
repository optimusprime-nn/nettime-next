//! RFC 8915 transport. Cookies and exporter keys stay in memory and are never serialized.
use crate::{config::Server, protocol::mono};
use aes_siv::{KeyInit, siv::Aes128Siv};
use anyhow::{Result, bail, ensure};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned, pki_types::ServerName};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::Arc,
    time::Duration,
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
            n >= 4 && n % 4 == 0 && pos + n <= data.len(),
            "Invalid extension length"
        );
        out.push((word(&data[pos..]), &data[pos + 4..pos + n], pos));
        pos += n;
    }
    Ok(out)
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
    let endpoint = (server.host.as_str(), server.nts_ke_port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow::anyhow!("No NTS-KE address"))?;
    let socket = TcpStream::connect_timeout(&endpoint, Duration::from_secs_f64(timeout))?;
    socket.set_read_timeout(Some(Duration::from_secs_f64(timeout)))?;
    socket.set_write_timeout(Some(Duration::from_secs_f64(timeout)))?;
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
                    !body.is_empty() && size % 4 == 0 && size <= 1024,
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
#[derive(Default)]
pub struct NtsClient {
    pub session: Option<Session>,
    pub failures: u32,
    pub retry_at: f64,
}
impl NtsClient {
    pub fn prepare(&mut self, s: &Server, timeout: f64) -> Result<()> {
        ensure!(mono() >= self.retry_at, "NTS key establishment backoff");
        if self
            .session
            .as_ref()
            .is_none_or(|n| n.cookies.is_empty() || mono() - n.established > 86400.)
        {
            self.session = Some(establish(s, timeout)?);
        }
        Ok(())
    }
    pub fn failed(&mut self) {
        self.failures = (self.failures + 1).min(30);
        self.retry_at = mono() + (10. * 1.5_f64.powi(self.failures as i32 - 1)).min(432000.);
        self.session = None;
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
    #[test]
    fn rejects_truncated_and_misaligned_extensions() {
        assert!(fields(&[1, 4, 0, 8, 0], 0).is_err());
        assert!(fields(&[1, 4, 0, 5, 0], 0).is_err());
    }
}
