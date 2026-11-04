use crate::{
    config::Config,
    protocol::Sample,
    protocol::{self, mono, utc},
    selection::{self, Peer},
    storage::{self, Store},
};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
pub struct Monitor {
    pub cfg: Config,
    pub peers: Vec<Peer>,
    pub store: Store,
    cycle: i64,
    last_wall: f64,
    last_mono: f64,
    /// When the sample filters were last cleared for a clock change
    /// (monotonic seconds). Replies measured earlier are not filtered.
    cleared: f64,
    next_prune: f64,
    clock: Option<crate::clock::Session>,
    instance: String,
    maintenance: Option<thread::JoinHandle<Result<Value>>>,
    last_clock_event: String,
    last_maintenance_problem: String,
    // Cycles (with their own decisions) and events held back while another
    // connection keeps the database locked.
    unsaved: std::collections::VecDeque<(i64, Vec<Sample>, Value)>,
    events: Vec<(f64, String, Value)>,
    blocked_since: Option<f64>,
    // Cycles dropped from a full queue: count, samples, first and last utc.
    lost: Option<(usize, usize, f64, f64)>,
    max_unsaved_cycles: usize,
    in_flight: Vec<InFlight>,
    stop: Arc<AtomicBool>,
}
/// A source query still running. It holds the source's NTS client; `state` is
/// the client's backoff (failures, retry time) for saving meanwhile.
struct InFlight {
    peer: usize,
    state: (u32, f64),
    worker: thread::JoinHandle<(Sample, crate::nts::NtsClient)>,
}
/// Bounds on what is held in memory while the database stays locked
/// (1024 cycles is about 18 hours at the shortest poll).
const MAX_UNSAVED_CYCLES: usize = 1024;
const MAX_UNSAVED_EVENTS: usize = 1024;
impl Monitor {
    pub fn new(cfg: Config, mut store: Store) -> Result<Self> {
        // The migration observer must never become a second system clock writer.
        ensure!(
            cfg.clock.mode == "observe",
            "Native migration collector is measurement-only; current service keeps clock ownership"
        );
        store.configure_logging(&cfg);
        let mut peers: Vec<_> = cfg
            .servers
            .iter()
            .map(|s| Peer::new(s.clone(), cfg.poll_seconds))
            .collect();
        let now = mono();
        let wall = utc();
        for (key, state) in store.states()? {
            if let Some(p) = peers.iter_mut().find(|p| p.server.key() == key) {
                p.interval = cfg
                    .poll_seconds
                    .max(state["interval"].as_f64().unwrap_or(0.));
                p.disabled = state["disabled"].as_str().unwrap_or("").into();
                p.reach = state["reach"].as_u64().unwrap_or(0) as u8;
                p.next_due = now + (state["next_utc"].as_f64().unwrap_or(0.) - wall).max(0.);
                p.nts.failures = state["nts_failures"].as_u64().unwrap_or(0) as u32;
                // Earlier versions saved backoffs of up to five days.
                p.nts.retry_at = now
                    + (state["nts_retry_utc"].as_f64().unwrap_or(0.) - wall)
                        .clamp(0., crate::nts::MAX_BACKOFF);
            }
        }
        let cycle = store.next_cycle()?;
        Ok(Self {
            cfg,
            peers,
            store,
            cycle,
            last_wall: wall,
            last_mono: now,
            cleared: f64::NEG_INFINITY,
            next_prune: now,
            clock: None,
            instance: String::new(),
            maintenance: None,
            last_clock_event: String::new(),
            last_maintenance_problem: String::new(),
            unsaved: Default::default(),
            events: Vec::new(),
            blocked_since: None,
            lost: None,
            max_unsaved_cycles: MAX_UNSAVED_CYCLES,
            in_flight: Vec::new(),
            stop: Arc::new(AtomicBool::new(false)),
        })
    }
    pub fn for_service(cfg: Config, store: Store, root: &std::path::Path) -> Result<Self> {
        let options = cfg.clock.clone();
        let mut observed = cfg.clone();
        observed.clock.mode = "observe".into();
        let mut monitor = Self::new(observed, store)?;
        let mut nonce = [0u8; 32];
        getrandom::fill(&mut nonce)
            .map_err(|e| anyhow::anyhow!("Cannot identify collector session: {e}"))?;
        monitor.instance = hex::encode(nonce);
        crate::windows::atomic_write(
            &root.join("data/service-instance.json"),
            &serde_json::to_vec(
                &json!({"id":monitor.instance,"pid":std::process::id(),"utc":utc()}),
            )?,
        )?;
        if options.mode == "slew" {
            monitor.clock = Some(crate::clock::Session::start(root, options)?);
            monitor.record(
                "clock_controller_started",
                json!({"mode":"slew","limits":cfg.clock}),
            );
        }
        monitor.cfg = cfg;
        Ok(monitor)
    }
    pub fn collect(&mut self) -> Result<Option<Value>> {
        if let Some(clock) = &self.clock {
            clock.check()?;
        }
        let now = mono();
        if self
            .maintenance
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            let outcome = self.maintenance.take().unwrap().join();
            let value = match outcome {
                Ok(Ok(value)) => value,
                Ok(Err(error)) => {
                    json!({"error":format!("{error:#}"),"retry":"next maintenance pass"})
                }
                Err(_) => json!({"error":"History maintenance worker ended unexpectedly"}),
            };
            self.maintenance_finished(value, now);
        }
        if now >= self.next_prune && self.maintenance.is_none() {
            if let Some(path) = self.store.db.path().map(std::path::PathBuf::from) {
                let days = self.cfg.retention_days;
                self.maintenance = Some(thread::spawn(move || {
                    Store::open(&path, false)?.maintain(days)
                }));
            }
            self.next_prune = now + 3600.;
        }
        let wall = utc();
        let step = (wall - self.last_wall) - (now - self.last_mono);
        if step.abs() > 0.05 {
            self.record(
                "local_clock_discontinuity",
                json!({"step_ms":step*1000.,"action":"Cleared sample filters"}),
            );
            for p in &mut self.peers {
                p.samples.clear()
            }
            self.cleared = now;
            if let Some(clock) = &self.clock {
                clock.update(&selection::Assessment::default(), now)?;
            }
        }
        self.last_wall = wall;
        self.last_mono = now;
        let timeout = self.cfg.timeout_seconds;
        for (index, p) in self.peers.iter_mut().enumerate() {
            if !p.disabled.is_empty()
                || now < p.next_due
                || self.in_flight.iter().any(|f| f.peer == index)
            {
                continue;
            }
            let (server, interval) = (p.server.clone(), p.interval);
            let mut nts = std::mem::take(&mut p.nts);
            self.in_flight.push(InFlight {
                peer: index,
                state: (nts.failures, nts.retry_at),
                worker: thread::spawn(move || {
                    let sample = protocol::query(&server, timeout, interval, &mut nts);
                    (sample, nts)
                }),
            });
        }
        // Wait for the queries, but not past twice the request timeout. A source
        // still answering is collected in a later round instead of holding back
        // every other source's result, or a service stop.
        let until = mono() + 2. * timeout;
        while self.in_flight.iter().any(|f| !f.worker.is_finished())
            && mono() < until
            && !self.stop.load(Ordering::Relaxed)
        {
            thread::sleep(Duration::from_millis(10));
        }
        let (mut done, running): (Vec<_>, Vec<_>) = std::mem::take(&mut self.in_flight)
            .into_iter()
            .partition(|f| f.worker.is_finished());
        self.in_flight = running;
        done.sort_by_key(|f| f.peer);
        let mut samples = Vec::with_capacity(done.len());
        for f in done {
            let (sample, nts) = f
                .worker
                .join()
                .map_err(|_| anyhow::anyhow!("NTP worker panicked"))?;
            let p = &mut self.peers[f.peer];
            p.nts = nts;
            p.accept(sample.clone());
            // A slow reply measured before the clock changed carries the old offset.
            let cleared = self.cleared;
            p.samples.retain(|s| s.mono >= cleared);
            p.next_due = mono() + p.interval;
            samples.push(sample);
        }
        if samples.is_empty() {
            return Ok(None);
        }
        let assessment = selection::assess(&self.peers, &self.cfg, mono());
        let clock_action = if let Some(clock) = &self.clock {
            let plan = clock.update(&assessment, mono())?;
            let action = serde_json::to_value(plan)?;
            let label = action["action"].as_str().unwrap_or("");
            if label == "step" {
                // Every filtered sample now holds the old offset. Start again,
                // and move the discontinuity baseline past this deliberate change.
                for p in &mut self.peers {
                    p.samples.clear()
                }
                self.cleared = mono();
                self.last_wall = utc();
                self.last_mono = self.cleared;
            }
            let signature = format!("{}:{}", label, action["reason"]);
            if self.cfg.diagnostic_logging
                || label == "slew"
                || label == "step"
                || signature != self.last_clock_event
            {
                let mut evidence = action.clone();
                evidence["selected"] = json!(assessment.selected);
                if self.cfg.diagnostic_logging {
                    evidence["assessment"] = serde_json::to_value(&assessment)?;
                }
                self.record("clock_correction", evidence);
                self.last_clock_event = signature;
            }
            action
        } else {
            json!({"action":"observe","reason":"Clock control is off; explicit takeover required"})
        };
        let mut servers = serde_json::Map::new();
        for p in &self.peers {
            let mut v = p.latest.as_ref().map(|s| s.data()).unwrap_or(Value::Null);
            if let Some(obj) = v.as_object_mut() {
                obj.insert("reach".into(), json!(format!("{:03o}", p.reach)));
                obj.insert(
                    "next_check_seconds".into(),
                    json!((p.next_due - mono()).max(0.)),
                );
                obj.insert(
                    "selection_reason".into(),
                    json!(assessment.reasons.get(&p.server.key())),
                );
            }
            servers.insert(p.server.key(), v);
        }
        let decision = json!({"utc":utc(),"collector_instance":self.instance,"policy":"native-service-v2","selected":assessment.selected,"confidence":"Source agreement; independence depends on configured groups","explanation":assessment.explanation,"synchronization":assessment,"servers":servers,"clock_action":clock_action});
        let states = self.peers.iter().enumerate().map(|(index, p)| {
            let (failures, retry_at) = self
                .in_flight
                .iter()
                .find(|f| f.peer == index)
                .map_or((p.nts.failures, p.nts.retry_at), |f| f.state);
            (p.server.key(), json!({"interval":p.interval,"disabled":p.disabled,"reach":p.reach,"next_utc":utc()+(p.next_due-mono()).max(0.),"nts_failures":failures,"nts_retry_utc":utc()+(retry_at-mono()).max(0.)}))
        }).collect::<Vec<_>>();
        self.persist(samples, &decision, &states)?;
        Ok(Some(decision))
    }
    /// Schedules the next pass and logs the outcome. A deferral or error that
    /// repeats unchanged is logged once, not on every retry.
    fn maintenance_finished(&mut self, value: Value, now: f64) {
        let problem = value
            .get("error")
            .or(value.get("reason"))
            .map(Value::to_string);
        if problem.is_some() {
            self.next_prune = now + 60.;
        }
        let signature = problem.unwrap_or_default();
        if signature.is_empty() || signature != self.last_maintenance_problem {
            self.record("history_maintenance", value);
        }
        self.last_maintenance_problem = signature;
    }
    /// Queues an event; it is written with the next measurement save.
    fn record(&mut self, kind: &str, value: Value) {
        if self.events.len() == MAX_UNSAVED_EVENTS {
            self.events.remove(0);
        }
        self.events.push((utc(), kind.into(), value));
    }
    fn flush_events(&mut self) -> Result<()> {
        while let Some((at, kind, value)) = self.events.first() {
            self.store.event_at(*at, kind, value)?;
            self.events.remove(0);
        }
        Ok(())
    }
    /// Writes queued events and cycles. A reader holding the database past the
    /// busy timeout defers the write to the next cycle instead of stopping the
    /// service; clock correction does not depend on history. Each deferred
    /// cycle keeps its own number, samples and decision.
    fn persist(
        &mut self,
        samples: Vec<Sample>,
        decision: &Value,
        states: &[(String, Value)],
    ) -> Result<()> {
        if self.unsaved.len() >= self.max_unsaved_cycles
            && let Some((_, dropped, old)) = self.unsaved.pop_front()
        {
            let at = old["utc"].as_f64().unwrap_or(0.);
            let lost = self.lost.get_or_insert((0, 0, at, at));
            lost.0 += 1;
            lost.1 += dropped.len();
            lost.3 = at;
        }
        self.unsaved
            .push_back((self.cycle, samples, decision.clone()));
        self.cycle += 1;
        let result = self.flush_events().and_then(|()| {
            let cycles: Vec<_> = self
                .unsaved
                .iter()
                .map(|(cycle, samples, decision)| (*cycle, samples.as_slice(), decision))
                .collect();
            self.store.save_cycles(&cycles, states)
        });
        match result {
            Ok(()) => {
                let saved = self.unsaved.len();
                self.unsaved.clear();
                if let Some(since) = self.blocked_since.take() {
                    let mut evidence = json!({"blocked_seconds":utc()-since,"deferred_cycles_saved":saved,"reason":"Another program held the history database locked"});
                    if let Some((cycles, samples, first, last)) = self.lost.take() {
                        // History between these times is incomplete.
                        evidence["lost_cycles"] = json!(cycles);
                        evidence["lost_samples"] = json!(samples);
                        evidence["lost_from_utc"] = json!(first);
                        evidence["lost_to_utc"] = json!(last);
                    }
                    self.record("history_write_resumed", evidence);
                }
                Ok(())
            }
            Err(e) if storage::is_busy(&e) => {
                self.blocked_since.get_or_insert_with(utc);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
    pub fn run(&mut self, stop: Arc<AtomicBool>) -> Result<()> {
        // Queries still running at a stop are abandoned with the process.
        self.stop = stop.clone();
        let mode = self.cfg.clock.mode.clone();
        self.record(
            "native_collector_started",
            json!({"version":env!("CARGO_PKG_VERSION"),"mode":mode}),
        );
        while !stop.load(Ordering::Relaxed) {
            self.collect()?;
            thread::sleep(Duration::from_millis(250));
        }
        self.record("native_collector_stopped", json!({}));
        match self.flush_events() {
            Err(e) if !storage::is_busy(&e) => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locked_database_defers_history_instead_of_failing() {
        let root = crate::test_dir("locked-history");
        let path = root.join("history.sqlite3");
        let store = Store::open(&path, false).unwrap();
        store.db.busy_timeout(Duration::from_millis(50)).unwrap();
        let mut monitor = Monitor::new(Config::default(), store).unwrap();
        let cycle = monitor.cycle;
        let sample = |host: &str| Sample {
            host: host.into(),
            status: "valid".into(),
            utc: utc(),
            ..Default::default()
        };
        // A reader holding a read transaction, as an export or report does.
        let reader = Store::open(&path, true).unwrap();
        reader.db.execute_batch("BEGIN").unwrap();
        let _: i64 = reader
            .db
            .query_row("SELECT count(*) FROM samples", [], |r| r.get(0))
            .unwrap();
        monitor.record("proof", json!({}));
        let decision = |selected: &str, at: f64| json!({"selected":selected,"utc":at});
        monitor
            .persist(vec![sample("a")], &decision("a", 1000.), &[])
            .unwrap();
        monitor
            .persist(vec![sample("b")], &decision("b", 1064.), &[])
            .unwrap();
        assert_eq!(monitor.unsaved.len(), 2);
        assert!(monitor.blocked_since.is_some());
        reader.db.execute_batch("COMMIT").unwrap();
        monitor
            .persist(vec![sample("c")], &decision("c", 1128.), &[])
            .unwrap();
        assert_eq!(monitor.cycle, cycle + 3);
        let count = |sql: &str| -> i64 { reader.db.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(count("SELECT count(*) FROM samples"), 3);
        assert_eq!(count("SELECT count(DISTINCT cycle) FROM samples"), 3);
        assert_eq!(count("SELECT count(*) FROM events WHERE kind='proof'"), 1);
        let mut q = reader
            .db
            .prepare("SELECT cycle,utc,selected FROM decisions ORDER BY cycle")
            .unwrap();
        let rows: Vec<(i64, f64, String)> = q
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            rows,
            [
                (cycle, 1000., "a".into()),
                (cycle + 1, 1064., "b".into()),
                (cycle + 2, 1128., "c".into())
            ]
        );
        drop(q);
        assert_eq!(monitor.events[0].1, "history_write_resumed");
        assert_eq!(monitor.events[0].2["deferred_cycles_saved"], 3);
        assert!(monitor.events[0].2.get("lost_cycles").is_none());
        drop((reader, monitor));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn slow_source_does_not_hold_back_the_round() {
        use crate::config::Server;
        use std::io::Write;
        // A plain NTP source on loopback that answers every request.
        let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let udp_port = udp.local_addr().unwrap().port();
        thread::spawn(move || {
            let mut request = [0u8; 1024];
            while let Ok((_, client)) = udp.recv_from(&mut request) {
                let now = protocol::encode(utc());
                let mut reply = [0u8; 48];
                reply[0] = 0x24;
                reply[1] = 2;
                reply[16..24].copy_from_slice(&protocol::encode(utc() - 1.));
                reply[24..32].copy_from_slice(&request[40..48]);
                reply[32..40].copy_from_slice(&now);
                reply[40..48].copy_from_slice(&now);
                let _ = udp.send_to(&reply, client);
            }
        });
        // An NTS-KE peer that trickles a TLS record until the deadline.
        let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let tcp_port = tcp.local_addr().unwrap().port();
        thread::spawn(move || {
            for mut s in tcp.incoming().flatten() {
                thread::spawn(move || {
                    let _ = s.write_all(&[0x16, 0x03, 0x03, 0x40, 0x00]);
                    while s.write_all(&[0]).is_ok() {
                        thread::sleep(Duration::from_millis(50));
                    }
                });
            }
        });
        let cfg = Config {
            timeout_seconds: 1.,
            servers: vec![
                Server {
                    host: "127.0.0.1".into(),
                    nts_ke_port: tcp_port,
                    security: "nts_required".into(),
                    ..Default::default()
                },
                Server {
                    host: "127.0.0.1".into(),
                    port: udp_port,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let store = Store::open(std::path::Path::new(":memory:"), false).unwrap();
        let mut monitor = Monitor::new(cfg, store).unwrap();
        // The key exchange may take up to 3 s; the round waits at most 2 s.
        let started = std::time::Instant::now();
        assert!(monitor.collect().unwrap().is_some());
        assert!(started.elapsed() < Duration::from_millis(2800));
        assert_eq!(monitor.peers[1].latest.as_ref().unwrap().status, "valid");
        assert!(monitor.peers[0].latest.is_none());
        assert_eq!(monitor.in_flight.len(), 1);
        // The slow source's outcome is collected by a later round, with its
        // NTS client (and the failure it counted) handed back.
        thread::sleep(Duration::from_millis(1500));
        assert!(monitor.collect().unwrap().is_some());
        assert!(monitor.in_flight.is_empty());
        let late = monitor.peers[0].latest.as_ref().unwrap();
        assert!(late.reason.contains("deadline"), "{}", late.reason);
        assert_eq!(monitor.peers[0].nts.failures, 1);
    }
    #[test]
    fn repeated_maintenance_problems_are_logged_once() {
        let store = Store::open(std::path::Path::new(":memory:"), false).unwrap();
        let mut monitor = Monitor::new(Config::default(), store).unwrap();
        let deferred =
            json!({"deferred":true,"reason":"Administrator transaction has not committed"});
        for _ in 0..5 {
            monitor.maintenance_finished(deferred.clone(), 100.);
        }
        assert_eq!(monitor.next_prune, 160.);
        monitor.maintenance_finished(json!({"error":"disk full"}), 200.);
        monitor.maintenance_finished(json!({"error":"disk full"}), 260.);
        monitor.maintenance_finished(json!({"compressed_records":3}), 320.);
        monitor.maintenance_finished(json!({"compressed_records":0}), 380.);
        monitor.maintenance_finished(deferred.clone(), 440.);
        let kinds: Vec<_> = monitor.events.iter().map(|e| e.2.clone()).collect();
        assert_eq!(kinds.len(), 5, "{kinds:?}");
        assert_eq!(kinds[0], deferred);
        assert_eq!(kinds[1]["error"], "disk full");
        assert_eq!(kinds[4], deferred);
    }
    #[test]
    fn full_deferral_queue_reports_the_lost_interval() {
        let root = crate::test_dir("lost-history");
        let path = root.join("history.sqlite3");
        let store = Store::open(&path, false).unwrap();
        store.db.busy_timeout(Duration::ZERO).unwrap();
        let mut monitor = Monitor::new(Config::default(), store).unwrap();
        monitor.max_unsaved_cycles = 8;
        let reader = Store::open(&path, true).unwrap();
        reader.db.execute_batch("BEGIN").unwrap();
        let _: i64 = reader
            .db
            .query_row("SELECT count(*) FROM samples", [], |r| r.get(0))
            .unwrap();
        for i in 0..10 {
            let sample = Sample {
                status: "valid".into(),
                ..Default::default()
            };
            monitor
                .persist(vec![sample], &json!({"utc":i as f64}), &[])
                .unwrap();
        }
        assert_eq!(monitor.unsaved.len(), 8);
        reader.db.execute_batch("COMMIT").unwrap();
        monitor.persist(vec![], &json!({"utc":9999.}), &[]).unwrap();
        let resumed = &monitor.events[0].2;
        assert_eq!(resumed["lost_cycles"], 3);
        assert_eq!(resumed["lost_samples"], 3);
        assert_eq!(resumed["lost_from_utc"], 0.);
        assert_eq!(resumed["lost_to_utc"], 2.);
        drop((reader, monitor));
        std::fs::remove_dir_all(root).unwrap();
    }
}
