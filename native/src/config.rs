use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub security: String,
    pub source_group: String,
    pub nts_ke_port: u16,
}
impl Default for Server {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 123,
            security: "plain_ntp".into(),
            source_group: String::new(),
            nts_ke_port: 4460,
        }
    }
}
impl Server {
    pub fn key(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Clock {
    pub mode: String,
    pub require_nts: bool,
    pub min_sources: usize,
    pub max_offset_ms: f64,
    pub max_slew_ppm: f64,
    pub holdover_seconds: f64,
}
impl Default for Clock {
    fn default() -> Self {
        Self {
            mode: "observe".into(),
            require_nts: false,
            min_sources: 3,
            max_offset_ms: 128.,
            max_slew_ppm: 250.,
            holdover_seconds: 900.,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub servers: Vec<Server>,
    pub poll_seconds: f64,
    pub timeout_seconds: f64,
    pub retention_days: f64,
    pub max_distance_ms: f64,
    pub clock: Clock,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            servers: vec![],
            poll_seconds: 64.,
            timeout_seconds: 3.,
            retention_days: 7.,
            max_distance_ms: 1000.,
            clock: Clock::default(),
        }
    }
}
impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let mut cfg: Self = serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
        cfg.validate()?;
        Ok(cfg)
    }
    pub fn validate(&mut self) -> Result<()> {
        ensure!(
            (1..=32).contains(&self.servers.len()),
            "Configure 1–32 servers"
        );
        let mut keys = HashSet::new();
        for s in &mut self.servers {
            ensure!(
                !s.host.is_empty() && s.host.trim() == s.host && s.host.len() <= 253,
                "Invalid hostname"
            );
            ensure!(s.port > 0 && s.nts_ke_port > 0, "Ports must be 1–65535");
            ensure!(
                keys.insert((s.host.to_lowercase(), s.port)),
                "Duplicate server"
            );
            ensure!(
                ["plain_ntp", "nts_required"].contains(&s.security.as_str()),
                "Unsupported security mode"
            );
            ensure!(s.source_group.len() <= 100, "Source group too long");
            s.source_group = s.source_group.trim().to_lowercase();
        }
        for (name, value, low, high) in [
            ("poll", self.poll_seconds, 64., 131072.),
            ("timeout", self.timeout_seconds, 0.1, 30.),
            ("retention", self.retention_days, 7., 3650.),
            ("distance", self.max_distance_ms, 1., 10000.),
            ("offset", self.clock.max_offset_ms, 1., 128.),
            ("slew", self.clock.max_slew_ppm, 1., 500.),
            ("holdover", self.clock.holdover_seconds, 64., 3600.),
        ] {
            ensure!(
                value.is_finite() && (low..=high).contains(&value),
                "{name} outside permitted range"
            );
        }
        ensure!(
            (3..=32).contains(&self.clock.min_sources),
            "Need 3–32 independent groups"
        );
        if !["observe", "slew"].contains(&self.clock.mode.as_str()) {
            bail!("Invalid clock mode")
        }
        Ok(())
    }
}
pub fn machine_root() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into()))
        .join("NetTimeNext")
}
