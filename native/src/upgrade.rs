//! Stop/resume only the native collector while keeping a clock ownership journal.
use crate::{
    ownership::{self, Services, Snapshot, WindowsServices},
    service,
    windows::atomic_write,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Serialize, Deserialize)]
struct Upgrade {
    phase: String,
    owned: bool,
    service: Snapshot,
}
pub fn pending() -> Result<bool> {
    pending_at(&service::root())
}
fn pending_at(root: &Path) -> Result<bool> {
    let path = root.join("upgrade-state.json");
    if !path.exists() {
        return Ok(false);
    }
    let state: Upgrade = serde_json::from_slice(&std::fs::read(path)?)?;
    Ok(state.phase == "prepared")
}
/// Run by the installer when setup ends without --finish-upgrade (cancelled
/// or failed after --prepare-upgrade). Restarts the collector if it was
/// running, with its existing registration, and clears the pending upgrade so
/// administrator actions are not blocked. Clock ownership is unchanged.
pub fn abort() -> Result<()> {
    abort_at(&service::root(), &mut WindowsServices)
}
fn abort_at(root: &Path, backend: &mut impl Services) -> Result<()> {
    if !pending_at(root)? {
        return Ok(());
    }
    let mut state: Upgrade =
        serde_json::from_slice(&std::fs::read(root.join("upgrade-state.json"))?)?;
    ensure!(
        ownership::active(root)? == state.owned,
        "Clock ownership changed during upgrade"
    );
    if state.service.running && backend.snapshot(service::NAME)?.exists {
        backend.change(service::NAME, true)?;
    }
    state.phase = "aborted".into();
    atomic_write(
        &root.join("upgrade-state.json"),
        &serde_json::to_vec_pretty(&state)?,
    )
}
pub fn prepare() -> Result<()> {
    let root = service::root();
    crate::settings_update::recover(&root, &mut WindowsServices)?;
    // Journals from older versions lack newer controllers; record and stop
    // them before the guard below requires them stopped.
    ownership::migrate(&root, &mut WindowsServices)?;
    if pending()? {
        let state: Upgrade =
            serde_json::from_slice(&std::fs::read(root.join("upgrade-state.json"))?)?;
        ensure!(
            ownership::active(&root)? == state.owned,
            "Clock ownership changed during interrupted upgrade"
        );
        if state.owned {
            ensure!(
                ownership::guard(&root, &mut WindowsServices)?,
                "Clock ownership conflict during upgrade"
            );
        }
        WindowsServices.change(service::NAME, false)?;
        return Ok(());
    }
    ensure!(
        ownership::journal(&root)?
            .is_none_or(|j| ["active", "rolled_back"].contains(&j.phase.as_str())),
        "Clock ownership restoration is incomplete"
    );
    let owned = ownership::active(&root)?;
    if owned {
        ensure!(
            ownership::guard(&root, &mut WindowsServices)?,
            "Clock ownership conflict; upgrade refused"
        );
        ensure!(
            crate::config::Config::load(&root.join("settings.json"))?
                .clock
                .mode
                == "slew",
            "Active clock configuration is invalid; upgrade refused"
        );
    }
    let own = WindowsServices.snapshot(service::NAME)?;
    let state = Upgrade {
        phase: "prepared".into(),
        owned,
        service: own.clone(),
    };
    atomic_write(
        &root.join("upgrade-state.json"),
        &serde_json::to_vec_pretty(&state)?,
    )?;
    if own.exists && own.running {
        WindowsServices.change(service::NAME, false)?;
    }
    Ok(())
}
pub fn finish() -> Result<()> {
    let root = service::root();
    let mut state: Upgrade =
        serde_json::from_slice(&std::fs::read(root.join("upgrade-state.json"))?)?;
    ensure!(state.phase == "prepared", "No pending service upgrade");
    ensure!(
        ownership::active(&root)? == state.owned,
        "Clock ownership changed during upgrade"
    );
    service::install()?;
    if state.service.running {
        WindowsServices.change(service::NAME, true)?;
    }
    state.phase = "complete".into();
    atomic_write(
        &root.join("upgrade-state.json"),
        &serde_json::to_vec_pretty(&state)?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake {
        own: Snapshot,
        starts: usize,
    }
    impl Services for Fake {
        fn snapshot(&mut self, _: &str) -> Result<Snapshot> {
            Ok(self.own.clone())
        }
        fn change(&mut self, _: &str, running: bool) -> Result<()> {
            self.starts += usize::from(running);
            self.own.running = running;
            Ok(())
        }
        fn configure(&mut self, _: &str, _: Option<u32>, _: Option<&str>) -> Result<()> {
            unreachable!("aborting an upgrade never reconfigures services")
        }
        fn privilege(&mut self, _: Option<bool>) -> Result<bool> {
            unreachable!()
        }
        fn neutral(&mut self) -> Result<()> {
            unreachable!()
        }
    }
    #[test]
    fn aborted_upgrade_restarts_a_previously_running_collector_once() {
        let root = crate::test_dir("abort-upgrade");
        std::fs::create_dir_all(&root).unwrap();
        let own = Snapshot {
            name: service::NAME.into(),
            exists: true,
            start_type: 2,
            account: "NT AUTHORITY\\LocalService".into(),
            running: true,
        };
        let state = Upgrade {
            phase: "prepared".into(),
            owned: false,
            service: own.clone(),
        };
        std::fs::write(
            root.join("upgrade-state.json"),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        let mut backend = Fake {
            own: Snapshot {
                running: false,
                ..own
            },
            starts: 0,
        };
        abort_at(&root, &mut backend).unwrap();
        assert!(backend.own.running);
        assert!(!pending_at(&root).unwrap());
        abort_at(&root, &mut backend).unwrap();
        assert_eq!(backend.starts, 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
