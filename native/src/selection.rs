use crate::{
    config::{Config, Server},
    protocol::Sample,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
pub struct Peer {
    pub server: Server,
    pub samples: VecDeque<Sample>,
    pub latest: Option<Sample>,
    pub reach: u8,
    pub interval: f64,
    pub next_due: f64,
    pub disabled: String,
    pub nts: crate::nts::NtsClient,
}
impl Peer {
    pub fn new(server: Server, interval: f64) -> Self {
        Self {
            server,
            samples: VecDeque::new(),
            latest: None,
            reach: 0,
            interval,
            next_due: 0.,
            disabled: String::new(),
            nts: Default::default(),
        }
    }
    pub fn accept(&mut self, s: Sample) {
        if self
            .latest
            .as_ref()
            .is_some_and(|last| last.address != s.address)
        {
            self.samples.clear();
        }
        self.reach = (self.reach << 1) | u8::from(s.status == "valid");
        if s.status == "valid" {
            if self.samples.len() == 8 {
                self.samples.pop_front();
            }
            self.samples.push_back(s.clone());
        }
        if s.status == "control" {
            match s.kod.as_str() {
                "RATE" => {
                    self.interval =
                        (self.interval * 2.).max(2_f64.powi((s.poll as i32).clamp(6, 17)))
                }
                "DENY" | "RSTR" => {
                    self.disabled = format!(
                        "Server {}: association stopped; operator review required",
                        s.kod
                    )
                }
                _ => {}
            }
        }
        self.latest = Some(s);
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub peer: String,
    pub address: String,
    pub port: u16,
    pub source_group: String,
    pub offset_ms: f64,
    pub jitter_ms: f64,
    pub distance_ms: f64,
    pub distance_components_ms: HashMap<String, f64>,
    pub stratum: u8,
    pub leap: u8,
    pub sample_mono: f64,
    pub sample_utc: f64,
    pub samples: usize,
    pub authentication: String,
    pub interval_seconds: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Assessment {
    pub policy: String,
    pub ready: bool,
    pub selected: Option<String>,
    pub offset_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub candidates: Vec<Candidate>,
    pub survivors: Vec<Candidate>,
    pub reasons: HashMap<String, String>,
    pub cluster_rounds: Vec<serde_json::Value>,
    pub explanation: String,
    pub sample_mono: f64,
    pub sample_utc: f64,
    pub interval_seconds: f64,
    pub intersection_ms: Option<[f64; 2]>,
    pub allowed_falsetickers: usize,
}
impl Default for Assessment {
    fn default() -> Self {
        Self {
            policy: "ntp-mitigation-preview-v1-rust".into(),
            ready: false,
            selected: None,
            offset_ms: None,
            jitter_ms: None,
            candidates: vec![],
            survivors: vec![],
            reasons: HashMap::new(),
            cluster_rounds: vec![],
            explanation: "Filters warming; no correction".into(),
            sample_mono: 0.,
            sample_utc: 0.,
            interval_seconds: 64.,
            intersection_ms: None,
            allowed_falsetickers: 0,
        }
    }
}
fn filtered(p: &Peer, now: f64, limit: f64) -> Result<Candidate, String> {
    if !p.disabled.is_empty() {
        return Err(p.disabled.clone());
    }
    let latest = p.latest.as_ref().ok_or("No current response")?;
    if latest.status != "valid" || p.reach == 0 {
        return Err(latest.reason.clone());
    }
    if now - latest.mono > 3. * p.interval {
        return Err("Latest response expired".into());
    }
    let mut samples: Vec<_> = p
        .samples
        .iter()
        .filter(|s| (0. ..=8. * p.interval + 1.).contains(&(now - s.mono)))
        .collect();
    samples.sort_by(|a, b| {
        a.delay_ms
            .total_cmp(&b.delay_ms)
            .then(b.mono.total_cmp(&a.mono))
    });
    let best = *samples.first().ok_or("Filter expired")?;
    if best.stratum > 1 && best.reference_id == best.local_address {
        return Err("Synchronization loop".into());
    }
    let dispersion: f64 = (0..8)
        .map(|i| {
            let epsilon = if let Some(s) = samples.get(i) {
                1. + 1000. * 2_f64.powi((s.precision as i32).clamp(-32, 0))
                    + 0.015 * (s.exchange_ms.max(0.) / 1000. + now - s.mono)
            } else {
                16000.
            };
            epsilon / 2_f64.powi(i as i32 + 1)
        })
        .sum();
    let jitter = (samples
        .iter()
        .map(|s| (s.offset_ms - best.offset_ms).powi(2))
        .sum::<f64>()
        / (samples.len().saturating_sub(1).max(1) as f64))
        .sqrt()
        .max(1.);
    let components = HashMap::from([
        ("half_network_delay_ms".into(), best.delay_ms / 2.),
        ("half_root_delay_ms".into(), best.root_delay_ms.max(0.) / 2.),
        ("root_dispersion_ms".into(), best.root_dispersion_ms),
        ("filter_dispersion_ms".into(), dispersion),
        ("jitter_ms".into(), jitter),
    ]);
    let distance = components.values().sum::<f64>().max(1.);
    if distance > limit {
        return Err(format!(
            "Root distance {distance:.3} ms exceeds limit; filter warming or source uncertain"
        ));
    }
    Ok(Candidate {
        peer: p.server.key(),
        address: best.address.clone(),
        port: best.remote_port,
        source_group: p.server.source_group.clone(),
        offset_ms: best.offset_ms,
        jitter_ms: jitter,
        distance_ms: distance,
        distance_components_ms: components,
        stratum: best.stratum,
        leap: best.leap,
        sample_mono: best.mono,
        sample_utc: best.utc,
        samples: samples.len(),
        authentication: best.authentication_status.clone(),
        interval_seconds: p.interval,
    })
}
fn intersection(c: &[Candidate]) -> Option<(f64, f64, usize)> {
    let mut endpoints: Vec<_> = c
        .iter()
        .flat_map(|c| [-1, 0, 1].map(|kind| (c.offset_ms + kind as f64 * c.distance_ms, kind)))
        .collect();
    endpoints.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    for f in 0..=(c.len() - 1) / 2 {
        let mut found = 0;
        let mut count = 0;
        let mut low = f64::INFINITY;
        let mut high = f64::NEG_INFINITY;
        for &(edge, kind) in &endpoints {
            count -= kind;
            if count >= (c.len() - f) as i32 {
                low = edge;
                break;
            }
            if kind == 0 {
                found += 1
            }
        }
        count = 0;
        for &(edge, kind) in endpoints.iter().rev() {
            count += kind;
            if count >= (c.len() - f) as i32 {
                high = edge;
                break;
            }
            if kind == 0 {
                found += 1
            }
        }
        if found <= f && low < high {
            return Some((low, high, f));
        }
    }
    None
}
pub fn assess(peers: &[Peer], cfg: &Config, now: f64) -> Assessment {
    let mut a = Assessment::default();
    let mut endpoints = HashSet::new();
    let mut groups = HashSet::new();
    for p in peers {
        let key = p.server.key();
        match filtered(p, now, cfg.max_distance_ms) {
            Err(reason) => {
                a.reasons.insert(key, reason);
            }
            Ok(c) => {
                let endpoint = (c.address.clone(), c.port);
                let reason = if endpoints.contains(&endpoint) {
                    "Same resolved endpoint; no additional vote"
                } else if c.source_group.is_empty() {
                    "Assign independent source group"
                } else if groups.contains(&c.source_group) {
                    "Same source group; measured but no additional vote"
                } else if c.samples < 4 {
                    "Filter warming: need four fresh measurements"
                } else if cfg.clock.require_nts && c.authentication != "authenticated" {
                    "Clock policy requires NTS"
                } else {
                    "Filtered candidate"
                };
                a.reasons.insert(key, reason.into());
                if reason == "Filtered candidate" {
                    endpoints.insert(endpoint);
                    groups.insert(c.source_group.clone());
                    a.candidates.push(c);
                }
            }
        }
    }
    if a.candidates.len() < cfg.clock.min_sources {
        a.explanation = format!(
            "Need {} eligible source groups; currently {}",
            cfg.clock.min_sources,
            a.candidates.len()
        );
        return a;
    }
    let Some((low, high, f)) = intersection(&a.candidates) else {
        a.explanation = "No majority correctness interval; synchronization withheld".into();
        return a;
    };
    a.intersection_ms = Some([low, high]);
    a.allowed_falsetickers = f;
    for c in &a.candidates {
        if low <= c.offset_ms && c.offset_ms <= high {
            a.survivors.push(c.clone());
        } else {
            a.reasons.insert(
                c.peer.clone(),
                "Falseticker: midpoint outside majority interval".into(),
            );
        }
    }
    a.survivors.sort_by(|x, y| {
        (x.stratum as f64 * cfg.max_distance_ms + x.distance_ms)
            .total_cmp(&(y.stratum as f64 * cfg.max_distance_ms + y.distance_ms))
            .then(x.peer.cmp(&y.peer))
    });
    let mut selection_jitter = 0.;
    while !a.survivors.is_empty() {
        let jitters: Vec<_> = a
            .survivors
            .iter()
            .map(|c| {
                (a.survivors
                    .iter()
                    .map(|q| (c.offset_ms - q.offset_ms).powi(2))
                    .sum::<f64>()
                    / a.survivors.len().saturating_sub(1).max(1) as f64)
                    .sqrt()
            })
            .collect();
        let worst = (0..jitters.len())
            .max_by(|&i, &j| jitters[i].total_cmp(&jitters[j]))
            .unwrap();
        selection_jitter = jitters[worst];
        if a.survivors.len() <= cfg.clock.min_sources
            || selection_jitter
                < a.survivors
                    .iter()
                    .map(|c| c.jitter_ms)
                    .fold(f64::INFINITY, f64::min)
        {
            break;
        }
        let removed = a.survivors.remove(worst);
        a.reasons.insert(
            removed.peer.clone(),
            "Cluster outlier: largest selection jitter".into(),
        );
        a.cluster_rounds.push(
            serde_json::json!({"removed":removed.peer,"selection_jitter_ms":selection_jitter}),
        );
    }
    if a.survivors.len() < cfg.clock.min_sources {
        a.explanation = "Insufficient survivors after mitigation".into();
        return a;
    }
    if a.survivors.iter().any(|c| c.leap != 0) {
        a.explanation = "Leap announcement/conflict: correction suspended".into();
        return a;
    }
    let weight: f64 = a.survivors.iter().map(|c| 1. / c.distance_ms).sum();
    let offset = a
        .survivors
        .iter()
        .map(|c| c.offset_ms / c.distance_ms)
        .sum::<f64>()
        / weight;
    let best = &a.survivors[0];
    a.selected = Some(best.peer.clone());
    a.offset_ms = Some(offset);
    a.jitter_ms = Some(selection_jitter.hypot(best.jitter_ms));
    a.sample_mono = best.sample_mono;
    a.sample_utc = best.sample_utc;
    a.interval_seconds = best.interval_seconds;
    a.ready = true;
    for (i, c) in a.survivors.iter().enumerate() {
        a.reasons.insert(
            c.peer.clone(),
            format!(
                "{}; weight {:.6}",
                if i == 0 {
                    "System peer"
                } else {
                    "Combined survivor"
                },
                1. / c.distance_ms / weight
            ),
        );
    }
    a.explanation = format!(
        "{} source groups survived; combined offset {offset:+.3} ms; system peer {}",
        a.survivors.len(),
        best.peer
    );
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    fn peers(count: usize) -> (Vec<Peer>, Config) {
        let mut cfg = Config::default();
        let peers = (0..count)
            .map(|i| {
                let s = Server {
                    host: format!("192.0.2.{}", i + 1),
                    source_group: format!("group{i}"),
                    ..Default::default()
                };
                cfg.servers.push(s.clone());
                let mut p = Peer::new(s.clone(), 64.);
                for j in 0..8 {
                    p.accept(Sample {
                        host: s.host.clone(),
                        port: 123,
                        remote_port: 123,
                        status: "valid".into(),
                        mono: 500. + j as f64 * 64.,
                        utc: 1700000000. + j as f64 * 64.,
                        address: s.host.clone(),
                        local_address: "192.0.2.200".into(),
                        offset_ms: 2. + i as f64 * 0.1,
                        delay_ms: 1.,
                        stratum: 1,
                        precision: -20,
                        authentication_status: "unauthenticated".into(),
                        ..Default::default()
                    });
                }
                p
            })
            .collect();
        (peers, cfg)
    }
    #[test]
    fn agreement_combines_independent_groups() {
        let (p, c) = peers(3);
        let a = assess(&p, &c, 949.);
        assert!(a.ready, "{}", a.explanation);
        assert!(a.offset_ms.unwrap() > 2. && a.offset_ms.unwrap() < 2.2);
    }
    #[test]
    fn same_group_cannot_multiply_votes() {
        let (mut p, mut c) = peers(3);
        p[2].server.source_group = "group1".into();
        c.servers[2].source_group = "group1".into();
        assert!(!assess(&p, &c, 949.).ready);
    }
    #[test]
    fn stale_or_unfilled_filters_cannot_control() {
        let (mut p, c) = peers(3);
        assert!(!assess(&p, &c, 1200.).ready);
        p[0].samples.truncate(1);
        assert!(!assess(&p, &c, 949.).ready);
    }
    #[test]
    fn falseticker_cannot_move_combined_time() {
        let (mut p, c) = peers(4);
        for s in &mut p[3].samples {
            s.offset_ms = 50000.;
        }
        let a = assess(&p, &c, 949.);
        assert!(a.ready);
        assert!(a.offset_ms.unwrap() < 3.);
        assert!(!a.survivors.iter().any(|s| s.peer == p[3].server.key()));
    }
    #[test]
    fn nts_required_policy_rejects_unauthenticated_votes() {
        let (p, mut c) = peers(3);
        c.clock.require_nts = true;
        assert!(!assess(&p, &c, 949.).ready);
    }
}
