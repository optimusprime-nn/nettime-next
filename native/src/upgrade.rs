//! Stop/resume only the native collector while keeping a clock ownership journal.
use crate::{
    ownership::{self, Services, Snapshot, WindowsServices},
    service,
    windows::atomic_write,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
struct Upgrade {
    phase: String,
    owned: bool,
    service: Snapshot,
}
pub fn pending() -> Result<bool> {
    let path = service::root().join("upgrade-state.json");
    if !path.exists() {
        return Ok(false);
    }
    let state: Upgrade = serde_json::from_slice(&std::fs::read(path)?)?;
    Ok(state.phase == "prepared")
}
pub fn prepare() -> Result<()> {
    let root = service::root();
    crate::settings_update::recover(&root, &mut WindowsServices)?;
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
