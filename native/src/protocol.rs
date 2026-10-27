use crate::{config::Server, nts::NtsClient};
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    net::{ToSocketAddrs, UdpSocket},
    sync::OnceLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
pub fn utc() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
pub fn mono() -> f64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}
pub fn encode(unix: f64) -> [u8; 8] {
    let n = unix + 2208988800.;
    let whole = n.floor();
    let mut a = [0; 8];
    a[..4].copy_from_slice(&(whole as u64 as u32).to_be_bytes());
    a[4..].copy_from_slice(&(((n - whole) * 4294967296.) as u32).to_be_bytes());
    a
}
pub fn decode(raw: &[u8], pivot: f64) -> f64 {
    let s = u32::from_be_bytes(raw[..4].try_into().unwrap()) as f64;
    let f = u32::from_be_bytes(raw[4..8].try_into().unwrap()) as f64 / 4294967296.;
    let base = s - 2208988800. + f;
    base + ((pivot - base) / 4294967296.).round() * 4294967296.
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Sample {
    pub host: String,
    pub port: u16,
    pub utc: f64,
    pub mono: f64,
    pub status: String,
    pub reason: String,
    pub address: String,
    pub local_address: String,
    pub source_port: u16,
    pub remote_port: u16,
    pub version: u8,
    pub mode: u8,
    pub stratum: u8,
    pub leap: u8,
    pub reference_id: String,
    pub reference_utc: f64,
    pub poll: i8,
    pub precision: i8,
    pub root_delay_ms: f64,
    pub root_dispersion_ms: f64,
    pub offset_ms: f64,
    pub delay_ms: f64,
    pub exchange_ms: f64,
    pub clock_step_ms: f64,
    pub t1: f64,
    pub t2: f64,
    pub t3: f64,
    pub t4: f64,
    pub request_hex: String,
    pub request_transmit_utc: f64,
    pub reply_hex: String,
    pub kod: String,
    pub security_mode: String,
    pub authentication_status: String,
    pub nts_ke_host: String,
    pub nts_ke_port: u16,
    pub tls_version: String,
    pub nts_key_exchange: String,
    pub nts_session_age_seconds: f64,
    pub nts_cookie_count: usize,
    pub certificate_sha256: String,
}
impl Sample {
    pub fn data(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).expect("finite measurement fields");
        if self.status != "valid" {
            value["offset_ms"] = serde_json::Value::Null;
            value["delay_ms"] = serde_json::Value::Null;
        }
        value
    }
}
pub fn parse(raw: &[u8], stamp: &[u8], s: &mut Sample) -> Result<()> {
    ensure!(raw.len() == 48, "NTP base header length must be 48");
    s.leap = raw[0] >> 6;
    s.version = (raw[0] >> 3) & 7;
    s.mode = raw[0] & 7;
    s.stratum = raw[1];
    s.poll = raw[2] as i8;
    s.precision = raw[3] as i8;
    s.root_delay_ms = i32::from_be_bytes(raw[4..8].try_into()?) as f64 / 65.536;
    s.root_dispersion_ms = u32::from_be_bytes(raw[8..12].try_into()?) as f64 / 65.536;
    s.reference_id = if s.stratum <= 1 {
        String::from_utf8_lossy(&raw[12..16])
            .trim_end_matches('\0')
            .into()
    } else if !s.address.contains(':') {
        format!("{}.{}.{}.{}", raw[12], raw[13], raw[14], raw[15])
    } else {
        hex::encode(&raw[12..16])
    };
    ensure!(
        s.mode == 4 && [3, 4].contains(&s.version),
        "Expected NTP v3/v4 server mode"
    );
    ensure!(&raw[24..32] == stamp, "Originate mismatch; possible replay");
    if s.stratum == 0 {
        s.kod = s.reference_id.clone();
        s.status = "control".into();
        ensure!(false, "Server control response: {}", s.kod)
    }
    ensure!(
        s.leap != 3 && (1..=15).contains(&s.stratum),
        "Source is unsynchronized"
    );
    ensure!(
        raw[16..24] != [0; 8] && raw[32..40] != [0; 8] && raw[40..48] != [0; 8],
        "Missing server timestamp"
    );
    s.t2 = decode(&raw[32..40], s.t1);
    s.t3 = decode(&raw[40..48], s.t1);
    s.reference_utc = decode(&raw[16..24], s.t3);
    ensure!(
        s.reference_utc <= s.t3 + 0.001 && s.t3 >= s.t2,
        "Invalid server timestamp ordering"
    );
    ensure!(
        s.clock_step_ms.abs() <= 5.,
        "Local clock changed during exchange"
    );
    let delay = (s.t4 - s.t1) - (s.t3 - s.t2);
    ensure!(
        delay >= -0.001_f64.max(2_f64.powi((s.precision as i32).clamp(-32, 0))),
        "Impossible negative round-trip delay"
    );
    s.delay_ms = (delay * 1000.).max(0.);
    s.offset_ms = ((s.t2 - s.t1) + (s.t3 - s.t4)) * 500.;
    s.status = "valid".into();
    s.reason = if s.authentication_status == "authenticated" {
        "NTS authentication and NTP header checks passed"
    } else {
        "NTP header checks passed; unauthenticated"
    }
    .into();
    Ok(())
}
pub fn query(server: &Server, timeout: f64, poll: f64, client: &mut NtsClient) -> Sample {
    let mut s = Sample {
        host: server.host.clone(),
        port: server.port,
        utc: utc(),
        mono: mono(),
        status: "error".into(),
        security_mode: server.security.clone(),
        authentication_status: if server.security == "nts_required" {
            "failed"
        } else {
            "unauthenticated"
        }
        .into(),
        ..Default::default()
    };
    let mut prepared = false;
    let result = (|| -> Result<()> {
        let secure = server.security == "nts_required";
        if secure {
            client.prepare(server, timeout)?;
            prepared = true;
        }
        let (host, port) = if secure {
            let n = client.session.as_ref().unwrap();
            (n.host.clone(), n.port)
        } else {
            (server.host.clone(), server.port)
        };
        let endpoint = (host.as_str(), port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| anyhow::anyhow!("No resolved address"))?;
        let socket = UdpSocket::bind(if endpoint.is_ipv6() {
            "[::]:0"
        } else {
            "0.0.0.0:0"
        })?;
        socket.connect(endpoint)?;
        socket.set_read_timeout(Some(Duration::from_secs_f64(timeout)))?;
        s.address = endpoint.ip().to_string();
        s.remote_port = port;
        s.local_address = socket.local_addr()?.ip().to_string();
        s.source_port = socket.local_addr()?.port();
        let mut header = [0u8; 48];
        header[0] = 0x23;
        header[2] = poll.log2().floor().clamp(6., 17.) as u8;
        header[3] = (-20i8) as u8;
        s.t1 = utc();
        let start = mono();
        let mut stamp = encode(s.t1);
        getrandom::fill(&mut stamp[7..]).map_err(|e| anyhow::anyhow!("Random source: {e}"))?;
        header[40..48].copy_from_slice(&stamp);
        s.request_hex = hex::encode(header);
        s.request_transmit_utc = decode(&stamp, s.t1);
        let mut uid = Vec::new();
        let mut cookie_limit = 0;
        let request = if secure {
            let n = client.session.as_mut().unwrap();
            let (data, id, limit) = n.request(&header)?;
            uid = id;
            cookie_limit = limit;
            data
        } else {
            header.to_vec()
        };
        socket.send(&request)?;
        let mut buffer = [0u8; 65536];
        // A datagram that is not this request's reply (wrong originate
        // timestamp, or failed NTS authentication) is discarded and the wait
        // continues, so a stray or spoofed packet cannot fail the poll.
        let mut discarded = 0;
        let mut last_discard = String::new();
        let unmatched = |discarded: usize, last: &str| {
            anyhow::anyhow!("No matching reply; discarded {discarded} datagrams ({last})")
        };
        loop {
            let remaining = timeout - (mono() - start);
            if remaining <= 0. {
                return Err(unmatched(discarded, &last_discard));
            }
            socket.set_read_timeout(Some(Duration::from_secs_f64(remaining.max(0.001))))?;
            let size = match socket.recv(&mut buffer) {
                Ok(size) => size,
                Err(e)
                    if discarded > 0
                        && matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                {
                    return Err(unmatched(discarded, &last_discard));
                }
                Err(e) => bail!(e),
            };
            s.t4 = utc();
            s.exchange_ms = (mono() - start) * 1000.;
            s.clock_step_ms = (s.t4 - s.t1) * 1000. - s.exchange_ms;
            s.utc = s.t4;
            s.mono = mono();
            s.reply_hex = hex::encode(&buffer[..size.min(48)]);
            if secure {
                let n = client.session.as_mut().unwrap();
                if let Err(e) = n.verify(&buffer[..size], &uid, cookie_limit) {
                    discarded += 1;
                    last_discard = e.to_string();
                    continue;
                }
                s.authentication_status = "authenticated".into();
                s.nts_ke_host = server.host.clone();
                s.nts_ke_port = server.nts_ke_port;
                s.tls_version = "TLSv1.3".into();
                s.nts_session_age_seconds = mono() - n.established;
                s.nts_cookie_count = n.cookies.len();
                s.certificate_sha256 = n.fingerprint.clone();
                ensure!(size >= 48, "Truncated authenticated NTP header");
                parse(&buffer[..48], &stamp, &mut s)?;
                client.exchange_succeeded();
            } else {
                if size < 32 || buffer[24..32] != stamp {
                    discarded += 1;
                    last_discard = "Originate mismatch".into();
                    continue;
                }
                parse(&buffer[..size], &stamp, &mut s)?;
            }
            return Ok(());
        }
    })();
    if let Err(e) = result {
        s.reason = e.to_string();
        // Key-establishment failures are counted inside prepare.
        if prepared && s.authentication_status != "authenticated" {
            client.exchange_failed();
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn era_round_trip() {
        for t in [0., 1730000000.125, 2085978497.5, 4100000000.25] {
            assert!((decode(&encode(t), t) - t).abs() < 0.000001)
        }
    }
    #[test]
    fn replay_and_unsynchronized_rejected() {
        let mut raw = [0u8; 48];
        raw[0] = 0x24;
        raw[1] = 1;
        let mut s = Sample::default();
        assert!(parse(&raw, &[1; 8], &mut s).is_err());
        raw[24..32].copy_from_slice(&[1; 8]);
        raw[0] |= 0xc0;
        assert!(parse(&raw, &[1; 8], &mut s).is_err());
    }
    /// An NTS-KE endpoint on loopback that closes every connection before TLS.
    fn refusing_key_exchange() -> Server {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                drop(stream);
            }
        });
        Server {
            host: "127.0.0.1".into(),
            nts_ke_port: port,
            security: "nts_required".into(),
            ..Default::default()
        }
    }
    #[test]
    fn polls_during_nts_backoff_do_not_extend_it() {
        let server = refusing_key_exchange();
        let mut client = NtsClient::default();
        let first = query(&server, 2., 64., &mut client);
        assert_eq!(first.status, "error");
        assert_eq!(client.failures, 1);
        let retry_at = client.retry_at;
        assert!(retry_at > mono());
        for _ in 0..20 {
            let refused = query(&server, 2., 64., &mut client);
            assert_eq!(refused.reason, "NTS key establishment backoff");
            assert_eq!(refused.authentication_status, "failed");
        }
        assert_eq!(client.failures, 1);
        assert_eq!(client.retry_at, retry_at);
    }
    #[test]
    fn nts_backoff_is_bounded() {
        let server = refusing_key_exchange();
        let mut client = NtsClient {
            failures: 29,
            ..Default::default()
        };
        query(&server, 2., 64., &mut client);
        assert_eq!(client.failures, 30);
        assert!(client.retry_at - mono() <= crate::nts::MAX_BACKOFF);
    }
    /// A loopback NTP server that first sends `junk` stray datagrams, then a
    /// valid reply if `answer` is set.
    fn stray_then_reply(junk: usize, answer: bool) -> Server {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut request = [0u8; 1024];
            let (_, client) = socket.recv_from(&mut request).unwrap();
            let now = encode(utc());
            let mut reply = [0u8; 48];
            reply[0] = 0x24;
            reply[1] = 2;
            reply[16..24].copy_from_slice(&encode(utc() - 1.));
            reply[32..40].copy_from_slice(&now);
            reply[40..48].copy_from_slice(&now);
            for _ in 0..junk {
                socket.send_to(&reply, client).unwrap();
            }
            if answer {
                reply[24..32].copy_from_slice(&request[40..48]);
                socket.send_to(&reply, client).unwrap();
            }
        });
        Server {
            host: "127.0.0.1".into(),
            port,
            ..Default::default()
        }
    }
    #[test]
    fn stray_datagrams_are_discarded_not_fatal() {
        let sample = query(
            &stray_then_reply(2, true),
            2.,
            64.,
            &mut NtsClient::default(),
        );
        assert_eq!(sample.status, "valid", "{}", sample.reason);
        assert_eq!(sample.stratum, 2);
    }
    #[test]
    fn only_stray_datagrams_time_out_with_their_reason() {
        let sample = query(
            &stray_then_reply(1, false),
            0.3,
            64.,
            &mut NtsClient::default(),
        );
        assert_eq!(sample.status, "error");
        assert!(sample.reason.contains("discarded 1"), "{}", sample.reason);
        assert!(
            sample.reason.contains("Originate mismatch"),
            "{}",
            sample.reason
        );
    }
    #[test]
    fn lost_nts_reply_keeps_session_until_repeated() {
        // A bound socket that never answers: every exchange times out.
        let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = silent.local_addr().unwrap().port();
        let server = Server {
            host: "127.0.0.1".into(),
            security: "nts_required".into(),
            ..Default::default()
        };
        let mut client = NtsClient {
            session: Some(crate::nts::Session::for_test("127.0.0.1", port, 8)),
            ..Default::default()
        };
        for lost in 1..crate::nts::MAX_MISSES {
            let sample = query(&server, 0.2, 64., &mut client);
            assert_eq!(sample.status, "error");
            let session = client.session.as_ref().expect("session kept");
            assert_eq!(session.cookies.len(), 8 - lost as usize);
            assert_eq!(client.failures, 0);
            assert_eq!(client.retry_at, 0.);
        }
        query(&server, 0.2, 64., &mut client);
        assert!(client.session.is_none());
        assert_eq!(client.failures, 1);
    }
    #[test]
    #[ignore = "Explicit external NTS interoperability check; two volleys 65 seconds apart"]
    fn live_nts_session_reuse() {
        std::thread::scope(|scope| {
            for host in ["time.cloudflare.com", "nts.netnod.se"] {
                scope.spawn(move || {
                    let server = Server {
                        host: host.into(),
                        security: "nts_required".into(),
                        ..Default::default()
                    };
                    let mut client = NtsClient::default();
                    let first = query(&server, 10., 64., &mut client);
                    assert_eq!(first.status, "valid", "{host}: {}", first.reason);
                    assert_eq!(first.authentication_status, "authenticated");
                    let established = client.session.as_ref().unwrap().established;
                    println!(
                        "{host}: authenticated first exchange; stratum {}",
                        first.stratum
                    );
                    std::thread::sleep(Duration::from_secs(65));
                    let second = query(&server, 10., 64., &mut client);
                    assert_eq!(second.status, "valid", "{host}: {}", second.reason);
                    assert_eq!(second.authentication_status, "authenticated");
                    assert_eq!(established, client.session.as_ref().unwrap().established);
                    assert!(second.nts_cookie_count > 0);
                    println!(
                        "{host}: authenticated reused session; {} cookies",
                        second.nts_cookie_count
                    );
                });
            }
        });
    }
}
