use crate::{
    config::Config,
    protocol::{self, mono, utc},
    selection::{self, Peer},
    storage::Store,
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
    next_prune: f64,
    clock: Option<crate::clock::Session>,
    instance: String,
    maintenance: Option<thread::JoinHandle<Result<Value>>>,
    last_clock_event: String,
}
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
                p.nts.retry_at =
                    now + (state["nts_retry_utc"].as_f64().unwrap_or(0.) - wall).max(0.);
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
            next_prune: now,
            clock: None,
            instance: String::new(),
            maintenance: None,
            last_clock_event: String::new(),
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
            monitor.store.event(
                "clock_controller_started",
                &json!({"mode":"slew","limits":cfg.clock}),
            )?;
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
            if value["deferred"] == true {
                self.next_prune = now + 5.;
            } else {
                if value.get("error").is_some() {
                    self.next_prune = now + 60.;
                }
                self.store.event("history_maintenance", &value)?;
            }
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
            self.store.event(
                "local_clock_discontinuity",
                &json!({"step_ms":step*1000.,"action":"Cleared sample filters"}),
            )?;
            for p in &mut self.peers {
                p.samples.clear()
            }
            if let Some(clock) = &self.clock {
                clock.update(&selection::Assessment::default(), now)?;
            }
        }
        self.last_wall = wall;
        self.last_mono = now;
        let timeout = self.cfg.timeout_seconds;
        let samples = thread::scope(|scope| {
            let workers: Vec<_> = self
                .peers
                .iter_mut()
                .filter(|p| p.disabled.is_empty() && now >= p.next_due)
                .map(|p| {
                    scope.spawn(move || {
                        let sample = protocol::query(&p.server, timeout, p.interval, &mut p.nts);
                        p.accept(sample.clone());
                        p.next_due = mono() + p.interval;
                        sample
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|w| w.join().map_err(|_| anyhow::anyhow!("NTP worker panicked")))
                .collect::<Result<Vec<_>>>()
        })?;
        if samples.is_empty() {
            return Ok(None);
        }
        let assessment = selection::assess(&self.peers, &self.cfg, mono());
        let clock_action = if let Some(clock) = &self.clock {
            let plan = clock.update(&assessment, mono())?;
            let action = serde_json::to_value(plan)?;
            let label = action["action"].as_str().unwrap_or("");
            let signature = format!("{}:{}", label, action["reason"]);
            if self.cfg.diagnostic_logging || label == "slew" || signature != self.last_clock_event
            {
                let mut evidence = action.clone();
                evidence["selected"] = json!(assessment.selected);
                if self.cfg.diagnostic_logging {
                    evidence["assessment"] = serde_json::to_value(&assessment)?;
                }
                self.store.event("clock_correction", &evidence)?;
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
        let states=self.peers.iter().map(|p|(p.server.key(),json!({"interval":p.interval,"disabled":p.disabled,"reach":p.reach,"next_utc":utc()+(p.next_due-mono()).max(0.),"nts_failures":p.nts.failures,"nts_retry_utc":utc()+(p.nts.retry_at-mono()).max(0.)}))).collect::<Vec<_>>();
        self.store.save(self.cycle, &samples, &decision, &states)?;
        self.cycle += 1;
        Ok(Some(decision))
    }
    pub fn run(&mut self, stop: Arc<AtomicBool>) -> Result<()> {
        self.store.event(
            "native_collector_started",
            &json!({"version":env!("CARGO_PKG_VERSION"),"mode":self.cfg.clock.mode}),
        )?;
        while !stop.load(Ordering::Relaxed) {
            self.collect()?;
            thread::sleep(Duration::from_millis(250));
        }
        self.store.event("native_collector_stopped", &json!({}))?;
        Ok(())
    }
}
