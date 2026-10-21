//! Administrator-only configuration transaction, separate from clock ownership.
use crate::{
    config::{Config, Server},
    ownership::{self, Services, Snapshot},
    service,
    storage::Store,
    windows::atomic_write,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path};

#[derive(Serialize, Deserialize)]
struct Update {
    phase: String,
    owned: bool,
    previous: Vec<u8>,
    service: Snapshot,
}
fn save(root: &Path, update: &Update) -> Result<()> {
    atomic_write(
        &root.join("settings-update.json"),
        &serde_json::to_vec_pretty(update)?,
    )
}
pub fn ownership_restored(root: &Path) -> Result<()> {
    let path = root.join("settings-update.json");
    if path.exists() {
        let mut update: Update = serde_json::from_slice(&std::fs::read(path)?)?;
        if !["complete", "rolled_back"].contains(&update.phase.as_str()) {
            update.phase = "rolled_back".into();
            save(root, &update)?;
        }
    }
    Ok(())
}
fn event(root: &Path, kind: &str, data: serde_json::Value) -> Result<()> {
    Store::open(&root.join("data/history.sqlite3"), false)?.event(kind, &data)
}
pub fn recover(root: &Path, backend: &mut impl Services) -> Result<()> {
    let path = root.join("settings-update.json");
    if !path.exists() {
        return Ok(());
    }
    let mut update: Update = serde_json::from_slice(&std::fs::read(path)?)?;
    if ["complete", "rolled_back"].contains(&update.phase.as_str()) {
        return Ok(());
    }
    ensure!(
        ownership::active(root)? == update.owned,
        "Pending settings update has a different clock ownership state; review it before continuing"
    );
    if update.owned {
        ensure!(
            ownership::guard(root, backend)?,
            "Clock ownership conflict during settings recovery"
        );
    }
    backend.change(service::NAME, false)?;
    atomic_write(&root.join("settings.json"), &update.previous)?;
    if update.service.running {
        backend.change(service::NAME, true)?;
    }
    update.phase = "rolled_back".into();
    save(root, &update)?;
    event(
        root,
        "settings_update_rolled_back",
        serde_json::json!({"ownership_preserved":update.owned}),
    )
}
fn same_endpoint(a: &Server, b: &Server) -> bool {
    a.host.eq_ignore_ascii_case(&b.host)
        && a.port == b.port
        && a.security == b.security
        && a.nts_ke_port == b.nts_ke_port
}
pub fn probe(server: &Server, timeout: f64, poll: f64) -> Result<()> {
    let source = server.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut client = crate::nts::NtsClient::default();
        let sample = crate::protocol::query(&source, timeout, poll, &mut client);
        let result = if sample.status == "valid"
            && (source.security != "nts_required"
                || sample.authentication_status == "authenticated")
        {
            Ok(())
        } else {
            Err(format!("{}: {}", sample.status, sample.reason))
        };
        let _ = tx.send(result);
    });
    let result = rx
        .recv_timeout(std::time::Duration::from_secs_f64(
            (timeout * 4. + 5.).clamp(8., 120.),
        ))
        .map_err(|_| anyhow::anyhow!("Source preflight timed out for {}", server.key()))?;
    result.map_err(|e| {
        anyhow::anyhow!(
            "Source preflight failed for {} ({}): {e}",
            server.key(),
            server.security
        )
    })
}
fn preflight(
    old: &Config,
    next: &Config,
    mut probe: impl FnMut(&Server) -> Result<()>,
) -> Result<()> {
    let groups: HashSet<_> = next
        .servers
        .iter()
        .filter(|s| {
            !s.source_group.is_empty() && (!next.clock.require_nts || s.security == "nts_required")
        })
        .map(|s| s.source_group.as_str())
        .collect();
    ensure!(
        groups.len() >= next.clock.min_sources,
        "Clock control requires {} eligible configured source groups; this update supplies {}",
        next.clock.min_sources,
        groups.len()
    );
    let mut verified_groups = HashSet::new();
    for source in &next.servers {
        let required = next.clock.require_nts && source.security == "nts_required";
        if required
            || !old
                .servers
                .iter()
                .any(|previous| same_endpoint(previous, source))
        {
            probe(source)?;
            if required && !source.source_group.is_empty() {
                verified_groups.insert(source.source_group.as_str());
            }
        }
    }
    if next.clock.require_nts {
        ensure!(
            verified_groups.len() >= next.clock.min_sources,
            "Clock policy requires {} independently grouped, verified NTS sources; found {}",
            next.clock.min_sources,
            verified_groups.len()
        );
    }
    Ok(())
}
pub fn apply(
    root: &Path,
    source: &Path,
    backend: &mut impl Services,
    probe: impl FnMut(&Server) -> Result<()>,
) -> Result<()> {
    ensure!(
        std::fs::metadata(source)?.len() <= 1_048_576,
        "Settings file exceeds 1 MB"
    );
    // Read the requested draft before recovering a previous interrupted update.
    let bytes = std::fs::read(source)?;
    let mut next: Config =
        serde_json::from_str(std::str::from_utf8(&bytes)?.trim_start_matches('\u{feff}'))?;
    next.validate()?;
    recover(root, backend)?;
    let owned = ownership::active(root)?;
    ensure!(
        ownership::journal(root)?
            .is_none_or(|j| ["active", "rolled_back"].contains(&j.phase.as_str())),
        "Clock ownership restoration is incomplete"
    );
    let own = backend.snapshot(service::NAME)?;
    ensure!(own.exists, "Register the native Windows service first");
    let previous = std::fs::read(root.join("settings.json"))?;
    let old = Config::load(&root.join("settings.json"))?;
    // A draft can neither grant takeover nor revoke an existing transaction.
    next.clock.mode = if owned { "slew" } else { "observe" }.into();
    if owned {
        ensure!(
            old.clock.mode == "slew" && own.running && ownership::guard(root, backend)?,
            "Active clock ownership must be healthy before applying settings"
        );
        if let Err(error) = preflight(&old, &next, probe) {
            event(
                root,
                "settings_preflight_rejected",
                serde_json::json!({"reason":format!("{error:#}"),"live_configuration_changed":false}),
            )?;
            return Err(error);
        }
        // Recheck after network waits, before any live state is changed.
        ensure!(
            ownership::guard(root, backend)?,
            "Clock ownership changed during source preflight"
        );
    }
    let mut update = Update {
        phase: "prepared".into(),
        owned,
        previous,
        service: own,
    };
    save(root, &update)?;
    let result = (|| -> Result<()> {
        if update.service.running {
            backend.change(service::NAME, false)?;
        }
        atomic_write(
            &root.join("settings.json"),
            &serde_json::to_vec_pretty(&next)?,
        )?;
        if update.service.running {
            backend.change(service::NAME, true)?;
        }
        event(
            root,
            "settings_applied",
            serde_json::json!({"clock_mode":next.clock.mode,"ownership_preserved":owned,"servers":next.servers,"retention_days":next.retention_days,"fresh_filters_required":owned}),
        )?;
        update.phase = "complete".into();
        save(root, &update)
    })();
    if let Err(error) = result {
        let restoration = recover(root, backend);
        anyhow::bail!(
            "Settings update failed: {error:#}. Previous settings restoration: {}",
            restoration.map_or_else(
                |e| format!("incomplete; retry the action: {e:#}"),
                |_| "completed".into()
            )
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Backend {
        own: Snapshot,
        fail_start: bool,
        changes: usize,
    }
    impl Services for Backend {
        fn snapshot(&mut self, name: &str) -> Result<Snapshot> {
            Ok(if name == service::NAME {
                self.own.clone()
            } else {
                Snapshot {
                    name: name.into(),
                    exists: true,
                    start_type: 4,
                    account: "LocalSystem".into(),
                    running: false,
                }
            })
        }
        fn change(&mut self, _: &str, running: bool) -> Result<()> {
            self.changes += 1;
            if running && self.fail_start {
                self.fail_start = false;
                anyhow::bail!("Injected restart failure");
            }
            self.own.running = running;
            Ok(())
        }
        fn configure(&mut self, _: &str, _: Option<u32>, _: Option<&str>) -> Result<()> {
            panic!("Settings must not change service identity or startup")
        }
        fn privilege(&mut self, _: Option<bool>) -> Result<bool> {
            panic!("Settings must not change clock privilege")
        }
        fn neutral(&mut self) -> Result<()> {
            panic!("Settings must not independently write the clock")
        }
    }
    fn fixture() -> (std::path::PathBuf, Config, Backend) {
        let root = std::env::current_dir().unwrap().join("work").join(format!(
            "nettime-settings-{}-{}",
            std::process::id(),
            crate::protocol::utc().to_bits()
        ));
        std::fs::create_dir_all(root.join("data")).unwrap();
        Store::open(&root.join("data/history.sqlite3"), false).unwrap();
        let own = Snapshot {
            name: service::NAME.into(),
            exists: true,
            start_type: 2,
            account: ownership::account(),
            running: true,
        };
        let mut cfg = Config::default();
        cfg.clock.mode = "slew".into();
        cfg.servers = ["one", "two", "three"]
            .iter()
            .map(|name| Server {
                host: format!("{name}.invalid"),
                source_group: (*name).into(),
                ..Server::default()
            })
            .collect();
        let bytes = serde_json::to_vec_pretty(&cfg).unwrap();
        atomic_write(&root.join("settings.json"), &bytes).unwrap();
        let journal = ownership::Journal {
            version: 1,
            phase: "active".into(),
            utc: crate::protocol::utc(),
            own_service: own.clone(),
            controllers: vec![],
            previous_config: b"original takeover snapshot".to_vec(),
            privilege_preexisting: false,
            clock_started: true,
        };
        atomic_write(
            &root.join("clock-ownership.json"),
            &serde_json::to_vec_pretty(&journal).unwrap(),
        )
        .unwrap();
        (
            root,
            cfg,
            Backend {
                own,
                fail_start: false,
                changes: 0,
            },
        )
    }
    #[test]
    fn active_update_keeps_ownership_and_tests_changed_nts_before_stopping() {
        let (root, mut cfg, mut backend) = fixture();
        let original = std::fs::read(root.join("clock-ownership.json")).unwrap();
        cfg.servers[0].security = "nts_required".into();
        cfg.clock.mode = "observe".into();
        let request = root.join("request.json");
        atomic_write(&request, &serde_json::to_vec(&cfg).unwrap()).unwrap();
        let mut probes = 0;
        apply(&root, &request, &mut backend, |s| {
            assert_eq!(s.security, "nts_required");
            probes += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(probes, 1);
        assert!(backend.own.running);
        assert_eq!(
            Config::load(&root.join("settings.json"))
                .unwrap()
                .clock
                .mode,
            "slew"
        );
        assert_eq!(
            std::fs::read(root.join("clock-ownership.json")).unwrap(),
            original
        );
    }
    #[test]
    fn failed_source_probe_makes_no_live_changes() {
        let (root, mut cfg, mut backend) = fixture();
        let previous = std::fs::read(root.join("settings.json")).unwrap();
        cfg.servers[0].security = "nts_required".into();
        let request = root.join("request.json");
        atomic_write(&request, &serde_json::to_vec(&cfg).unwrap()).unwrap();
        assert!(
            apply(&root, &request, &mut backend, |_| anyhow::bail!(
                "Certificate mismatch"
            ))
            .is_err()
        );
        assert_eq!(backend.changes, 0);
        assert_eq!(std::fs::read(root.join("settings.json")).unwrap(), previous);
    }
    #[test]
    fn failed_restart_restores_exact_settings_without_releasing_ownership() {
        let (root, mut cfg, mut backend) = fixture();
        let previous = std::fs::read(root.join("settings.json")).unwrap();
        cfg.retention_days = 14.;
        backend.fail_start = true;
        let request = root.join("request.json");
        atomic_write(&request, &serde_json::to_vec(&cfg).unwrap()).unwrap();
        assert!(
            apply(&root, &request, &mut backend, |_| panic!(
                "Unchanged endpoint should not be probed"
            ))
            .is_err()
        );
        assert!(backend.own.running);
        assert!(ownership::active(&root).unwrap());
        assert_eq!(std::fs::read(root.join("settings.json")).unwrap(), previous);
    }
    #[test]
    fn interrupted_update_is_recovered_and_global_nts_needs_enough_groups() {
        let (root, mut cfg, mut backend) = fixture();
        let previous = std::fs::read(root.join("settings.json")).unwrap();
        let update = Update {
            phase: "prepared".into(),
            owned: true,
            previous: previous.clone(),
            service: backend.own.clone(),
        };
        save(&root, &update).unwrap();
        atomic_write(&root.join("settings.json"), b"interrupted").unwrap();
        recover(&root, &mut backend).unwrap();
        assert_eq!(std::fs::read(root.join("settings.json")).unwrap(), previous);
        cfg.clock.require_nts = true;
        cfg.servers[0].security = "nts_required".into();
        assert!(preflight(&cfg, &cfg, |_| Ok(())).is_err());
    }
}
