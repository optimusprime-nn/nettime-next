use crate::{config::Server, nts::NtsClient};
use anyhow::{Result, ensure};
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
    let result = (|| -> Result<()> {
        let secure = server.security == "nts_required";
        if secure {
            client.prepare(server, timeout)?;
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
        let size = socket.recv(&mut buffer)?;
        s.t4 = utc();
        s.exchange_ms = (mono() - start) * 1000.;
        s.clock_step_ms = (s.t4 - s.t1) * 1000. - s.exchange_ms;
        s.utc = s.t4;
        s.mono = mono();
        s.reply_hex = hex::encode(&buffer[..size.min(48)]);
        if secure {
            let n = client.session.as_mut().unwrap();
            n.verify(&buffer[..size], &uid, cookie_limit)?;
            s.authentication_status = "authenticated".into();
            s.nts_ke_host = server.host.clone();
            s.nts_ke_port = server.nts_ke_port;
            s.tls_version = "TLSv1.3".into();
            s.nts_session_age_seconds = mono() - n.established;
            s.nts_cookie_count = n.cookies.len();
            s.certificate_sha256 = n.fingerprint.clone();
            ensure!(size >= 48, "Truncated authenticated NTP header");
            parse(&buffer[..48], &stamp, &mut s)?;
            client.failures = 0;
            client.retry_at = 0.;
        } else {
            parse(&buffer[..size], &stamp, &mut s)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        s.reason = e.to_string();
        if server.security == "nts_required" && s.authentication_status != "authenticated" {
            client.failed();
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
