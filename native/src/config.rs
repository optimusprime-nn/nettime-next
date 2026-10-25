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
    #[serde(deserialize_with = "port_number")]
    pub port: u16,
    pub security: String,
    pub source_group: String,
    #[serde(deserialize_with = "port_number")]
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
    #[serde(deserialize_with = "source_count")]
    pub min_sources: usize,
    pub max_offset_ms: f64,
    pub max_recovery_offset_ms: f64,
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
            max_recovery_offset_ms: 10_000.,
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
    pub logging_enabled: bool,
    pub diagnostic_logging: bool,
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
            logging_enabled: true,
            diagnostic_logging: false,
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
            ("retention", self.retention_days, 1., 3650.),
            ("distance", self.max_distance_ms, 1., 10000.),
            ("offset", self.clock.max_offset_ms, 1., 128.),
            (
                "recovery offset",
                self.clock.max_recovery_offset_ms,
                1.,
                60_000.,
            ),
            ("slew", self.clock.max_slew_ppm, 1., 500.),
            ("holdover", self.clock.holdover_seconds, 64., 3600.),
        ] {
            ensure!(
                value.is_finite() && (low..=high).contains(&value),
                "{name} outside permitted range"
            );
        }
        ensure!(
            self.clock.max_recovery_offset_ms >= self.clock.max_offset_ms,
            "Recovery limit must be at least the normal tracking limit"
        );
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

fn whole_number<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<u64, D::Error> {
    let number = serde_json::Number::deserialize(d)?;
    if let Some(value) = number.as_u64() {
        return Ok(value);
    }
    let value = number
        .as_f64()
        .ok_or_else(|| serde::de::Error::custom("Expected a whole number"))?;
    if value.is_finite() && (0. ..=65535.).contains(&value) && value.fract() == 0. {
        Ok(value as u64)
    } else {
        Err(serde::de::Error::custom(
            "Expected a nonnegative whole number",
        ))
    }
}
fn port_number<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<u16, D::Error> {
    u16::try_from(whole_number(d)?).map_err(serde::de::Error::custom)
}
fn source_count<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<usize, D::Error> {
    usize::try_from(whole_number(d)?).map_err(serde::de::Error::custom)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_defaults_have_public_authenticated_groups_and_do_not_take_over() {
        let mut cfg: Config =
            serde_json::from_str(include_str!("../settings.default.json")).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.clock.mode, "observe");
        assert_eq!(cfg.retention_days, 7.);
        assert!(cfg.logging_enabled && !cfg.diagnostic_logging);
        assert!(
            cfg.servers.iter().all(
                |s| s.security == "nts_required" && s.host.parse::<std::net::IpAddr>().is_err()
            )
        );
        assert!(
            cfg.servers
                .iter()
                .map(|s| &s.source_group)
                .collect::<HashSet<_>>()
                .len()
                >= cfg.clock.min_sources
        );
    }
    #[test]
    fn imports_legacy_integral_float_counts_and_ports_without_rounding() {
        let mut cfg:Config=serde_json::from_str(r#"{"servers":[{"host":"ntp.invalid","port":123.0,"nts_ke_port":4460.0}],"clock":{"min_sources":3.0}}"#).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.clock.min_sources, 3);
        assert_eq!(cfg.servers[0].port, 123);
        for value in ["3.5", "-1", "true", "\"3\""] {
            assert!(
                serde_json::from_str::<Config>(&format!(
                    r#"{{"clock":{{"min_sources":{value}}}}}"#
                ))
                .is_err()
            );
        }
        assert!(
            serde_json::from_str::<Config>(r#"{"servers":[{"host":"ntp.invalid","port":65536}]}"#)
                .is_err()
        );
    }
    #[test]
    fn existing_settings_gain_bounded_recovery_and_validate_its_limit() {
        let mut cfg: Config = serde_json::from_str(
            r#"{"servers":[{"host":"ntp.invalid"}],"clock":{"mode":"slew","max_offset_ms":128}}"#,
        )
        .unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.clock.max_recovery_offset_ms, 10_000.);
        assert_eq!(cfg.clock.mode, "slew");
        for invalid in [0., 127., 60_001., f64::NAN] {
            cfg.clock.max_recovery_offset_ms = invalid;
            assert!(cfg.validate().is_err());
        }
        cfg.clock.max_recovery_offset_ms = 128.;
        cfg.validate().unwrap();
    }
}
