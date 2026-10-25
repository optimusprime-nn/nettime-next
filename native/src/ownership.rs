//! Durable, explicit administrative ownership. No automatic handover on install.
use crate::{
    clock::{Backend, WindowsClock},
    config::Config,
    protocol::utc,
    service,
    storage::Store,
    windows::{atomic_write, check, wide},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    path::Path,
    time::{Duration, Instant},
};
use windows_service::{
    service::{ServiceAccess, ServiceState},
    service_manager::{ServiceManager, ServiceManagerAccess},
};
use windows_sys::Win32::{
    Security::{Authentication::Identity::*, LookupAccountNameW},
    System::Services::{ChangeServiceConfigW, SERVICE_NO_CHANGE},
};

pub const CONTROLLERS: [&str; 3] = ["NetTimeSvc", "W32Time", "NetTimeNext"];
pub fn account() -> String {
    format!("NT SERVICE\\{}", service::NAME)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub name: String,
    pub exists: bool,
    pub start_type: u32,
    pub account: String,
    pub running: bool,
}
pub trait Services {
    fn snapshot(&mut self, name: &str) -> Result<Snapshot>;
    fn identity(&mut self, name: &str) -> Result<String> {
        Ok(self.snapshot(name)?.account)
    }
    fn change(&mut self, name: &str, running: bool) -> Result<()>;
    fn configure(&mut self, name: &str, start: Option<u32>, account: Option<&str>) -> Result<()>;
    fn privilege(&mut self, grant: Option<bool>) -> Result<bool>;
    fn neutral(&mut self) -> Result<()>;
}
pub struct WindowsServices;
impl WindowsServices {
    fn open(
        name: &str,
        access: ServiceAccess,
    ) -> windows_service::Result<windows_service::service::Service> {
        ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?
            .open_service(name, access)
    }
}
impl Services for WindowsServices {
    fn identity(&mut self, name: &str) -> Result<String> {
        Ok(Self::open(name, ServiceAccess::QUERY_CONFIG)?
            .query_config()?
            .account_name
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned())
    }
    fn snapshot(&mut self, name: &str) -> Result<Snapshot> {
        let service = match Self::open(
            name,
            ServiceAccess::QUERY_CONFIG | ServiceAccess::QUERY_STATUS,
        ) {
            Ok(s) => s,
            Err(windows_service::Error::Winapi(e)) if e.raw_os_error() == Some(1060) => {
                return Ok(Snapshot {
                    name: name.into(),
                    exists: false,
                    start_type: 0,
                    account: String::new(),
                    running: false,
                });
            }
            Err(e) => return Err(e.into()),
        };
        let cfg = service.query_config()?;
        let state = service.query_status()?.current_state;
        ensure!(
            [ServiceState::Running, ServiceState::Stopped].contains(&state),
            "{name} is changing state; wait before proceeding"
        );
        Ok(Snapshot {
            name: name.into(),
            exists: true,
            start_type: cfg.start_type.to_raw(),
            account: cfg
                .account_name
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            running: state == ServiceState::Running,
        })
    }
    fn change(&mut self, name: &str, running: bool) -> Result<()> {
        let s = Self::open(
            name,
            ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::STOP,
        )?;
        let target = if running {
            ServiceState::Running
        } else {
            ServiceState::Stopped
        };
        let state = s.query_status()?.current_state;
        if state == target {
            return Ok(());
        }
        if running {
            s.start::<&str>(&[])?;
        } else if state != ServiceState::StopPending {
            s.stop()?;
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        while s.query_status()?.current_state != target {
            ensure!(Instant::now() < deadline, "{name} did not reach {target:?}");
            if running {
                ensure!(
                    s.query_status()?.current_state != ServiceState::Stopped,
                    "{name} failed to start; see service-error.log"
                );
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Ok(())
    }
    fn configure(&mut self, name: &str, start: Option<u32>, identity: Option<&str>) -> Result<()> {
        let s = Self::open(name, ServiceAccess::CHANGE_CONFIG)?;
        let identity = identity.map(wide);
        let empty = wide("");
        let password = if identity.as_ref().is_some_and(|s| {
            String::from_utf16_lossy(s)
                .to_lowercase()
                .starts_with("nt service\\")
        }) {
            std::ptr::null()
        } else {
            empty.as_ptr()
        };
        check(unsafe {
            ChangeServiceConfigW(
                s.raw_handle(),
                SERVICE_NO_CHANGE,
                start.unwrap_or(SERVICE_NO_CHANGE),
                SERVICE_NO_CHANGE,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                identity.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                if identity.is_some() {
                    password
                } else {
                    std::ptr::null()
                },
                std::ptr::null(),
            )
        })
    }
    fn privilege(&mut self, grant: Option<bool>) -> Result<bool> {
        service_privilege(grant)
    }
    fn neutral(&mut self) -> Result<()> {
        WindowsClock::new()?.restore()
    }
}
fn ntcheck(status: i32) -> Result<()> {
    if status != 0 {
        return Err(std::io::Error::from_raw_os_error(
            unsafe { LsaNtStatusToWinError(status) } as i32
        )
        .into());
    }
    Ok(())
}
fn service_privilege(grant: Option<bool>) -> Result<bool> {
    unsafe {
        let name = wide(account());
        let (mut size, mut domain_size, mut use_kind) = (0, 0, 0);
        LookupAccountNameW(
            std::ptr::null(),
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            &mut domain_size,
            &mut use_kind,
        );
        ensure!(size > 0, "Cannot resolve native virtual service account");
        // u32 storage keeps SID data aligned.
        let mut sid = vec![0u32; (size as usize).div_ceil(4)];
        let mut domain = vec![0u16; domain_size as usize];
        check(LookupAccountNameW(
            std::ptr::null(),
            name.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut size,
            domain.as_mut_ptr(),
            &mut domain_size,
            &mut use_kind,
        ))?;
        let attrs = LSA_OBJECT_ATTRIBUTES {
            Length: std::mem::size_of::<LSA_OBJECT_ATTRIBUTES>() as u32,
            ..Default::default()
        };
        let mut raw: LSA_HANDLE = 0;
        ntcheck(LsaOpenPolicy(
            std::ptr::null(),
            &attrs,
            (POLICY_LOOKUP_NAMES | POLICY_CREATE_ACCOUNT) as u32,
            &mut raw,
        ))?;
        struct Policy(LSA_HANDLE);
        impl Drop for Policy {
            fn drop(&mut self) {
                unsafe {
                    LsaClose(self.0);
                }
            }
        }
        let policy = Policy(raw);
        let (mut rights, mut count) = (std::ptr::null_mut(), 0);
        let status = LsaEnumerateAccountRights(
            policy.0,
            sid.as_ptr().cast_mut().cast(),
            &mut rights,
            &mut count,
        );
        let error = LsaNtStatusToWinError(status);
        if status != 0 && error != 2 && error != 259 {
            ntcheck(status)?;
        }
        let had = if status == 0 && count > 0 && !rights.is_null() {
            let had = std::slice::from_raw_parts(rights, count as usize)
                .iter()
                .any(|r| {
                    r.Length > 0
                        && !r.Buffer.is_null()
                        && String::from_utf16_lossy(std::slice::from_raw_parts(
                            r.Buffer,
                            r.Length as usize / 2,
                        )) == "SeSystemtimePrivilege"
                });
            LsaFreeMemory(rights.cast());
            had
        } else {
            if !rights.is_null() {
                LsaFreeMemory(rights.cast());
            }
            false
        };
        let mut text = wide("SeSystemtimePrivilege");
        let right = LSA_UNICODE_STRING {
            Length: ((text.len() - 1) * 2) as u16,
            MaximumLength: (text.len() * 2) as u16,
            Buffer: text.as_mut_ptr(),
        };
        match grant {
            Some(true) if !had => ntcheck(LsaAddAccountRights(
                policy.0,
                sid.as_ptr().cast_mut().cast(),
                &right,
                1,
            ))?,
            Some(false) if had => ntcheck(LsaRemoveAccountRights(
                policy.0,
                sid.as_ptr().cast_mut().cast(),
                false,
                &right,
                1,
            ))?,
            _ => {}
        }
        Ok(had)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Journal {
    pub version: u32,
    pub phase: String,
    pub utc: f64,
    pub own_service: Snapshot,
    pub controllers: Vec<Snapshot>,
    pub previous_config: Vec<u8>,
    pub privilege_preexisting: bool,
    pub clock_started: bool,
}
pub fn journal(root: &Path) -> Result<Option<Journal>> {
    let path = root.join("clock-ownership.json");
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&std::fs::read(path)?)?))
}
fn save(root: &Path, journal: &Journal) -> Result<()> {
    atomic_write(
        &root.join("clock-ownership.json"),
        &serde_json::to_vec_pretty(journal)?,
    )
}
pub fn active(root: &Path) -> Result<bool> {
    Ok(journal(root)?.is_some_and(|j| j.phase == "active"))
}
pub fn guard(root: &Path, backend: &mut impl Services) -> Result<bool> {
    if !active(root)? {
        return Ok(false);
    }
    if !backend
        .identity(service::NAME)?
        .eq_ignore_ascii_case(&account())
    {
        return Ok(false);
    }
    for name in CONTROLLERS {
        if backend.snapshot(name)?.running {
            return Ok(false);
        }
    }
    Ok(true)
}
pub fn readiness(cfg: &Config, decision: &serde_json::Value, now: f64) -> Result<()> {
    let a = &decision["synchronization"];
    ensure!(
        a["ready"].as_bool() == Some(true),
        "Takeover not ready: {}",
        a["explanation"]
    );
    let age = now - decision["utc"].as_f64().unwrap_or(0.);
    let sample_age = now - a["sample_utc"].as_f64().unwrap_or(0.);
    let offset = a["offset_ms"].as_f64().unwrap_or(f64::NAN);
    ensure!(
        (0. ..=180_f64.min(2. * cfg.poll_seconds)).contains(&age),
        "Collector evidence is stale"
    );
    ensure!(
        (0. ..=cfg.clock.holdover_seconds).contains(&sample_age),
        "Filtered sample expired"
    );
    ensure!(
        offset.is_finite() && offset.abs() <= cfg.clock.max_recovery_offset_ms,
        "Offset exceeds automatic recovery limit; no clock step"
    );
    Ok(())
}
pub fn activate(root: &Path, backend: &mut impl Services) -> Result<()> {
    ensure!(
        journal(root)?.is_none_or(|j| j.phase == "rolled_back"),
        "Restore the previous ownership transaction first"
    );
    let mut cfg = Config::load(&root.join("settings.json"))?;
    ensure!(
        cfg.clock.mode == "observe",
        "Service must start in observe mode"
    );
    let store = Store::open(&root.join("data/history.sqlite3"), true)?;
    let decision = store.latest()?.context("Wait for source measurements")?;
    let instance: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("data/service-instance.json"))?)?;
    ensure!(instance["id"].as_str().is_some_and(|id|!id.is_empty()&&decision["collector_instance"].as_str()==Some(id)),"Wait for fresh measurements from the current native service instance");
    readiness(&cfg, &decision, utc())?;
    let own = backend.snapshot(service::NAME)?;
    ensure!(
        own.exists && own.running,
        "Native service must be running before takeover"
    );
    ensure!(
        own.account
            .eq_ignore_ascii_case("NT AUTHORITY\\LocalService")
            || own.account.eq_ignore_ascii_case(&account()),
        "Custom service accounts are not changed"
    );
    let mut j = Journal {
        version: 1,
        phase: "prepared".into(),
        utc: utc(),
        own_service: own,
        controllers: CONTROLLERS
            .into_iter()
            .map(|n| backend.snapshot(n))
            .collect::<Result<_>>()?,
        previous_config: std::fs::read(root.join("settings.json"))?,
        privilege_preexisting: backend.privilege(None)?,
        clock_started: false,
    };
    save(root, &j)?;
    drop(store);
    Store::open(&root.join("data/history.sqlite3"), false)?.event(
        "takeover_prepared",
        &json!({"services":j.controllers,"assessment":decision["synchronization"]}),
    )?;
    let result = (|| -> Result<()> {
        backend.change(service::NAME, false)?;
        backend.privilege(Some(true))?;
        for s in &j.controllers {
            if s.exists {
                backend.change(&s.name, false)?;
                backend.configure(&s.name, Some(4), None)?;
            }
        }
        backend.configure(service::NAME, None, Some(&account()))?;
        cfg.clock.mode = "slew".into();
        atomic_write(
            &root.join("settings.json"),
            &serde_json::to_vec_pretty(&cfg)?,
        )?;
        j.phase = "active".into();
        j.clock_started = true;
        save(root, &j)?;
        backend.change(service::NAME, true)?;
        Store::open(&root.join("data/history.sqlite3"), false)?.event(
            "takeover_active",
            &json!({"services":j.controllers,"limits":cfg.clock}),
        )?;
        Ok(())
    })();
    if let Err(error) = result {
        let restoration = rollback(root, backend);
        anyhow::bail!(
            "Takeover failed: {error:#}. Restoration: {}",
            restoration.map_or_else(
                |e| format!("incomplete; retry restore: {e:#}"),
                |_| "completed".into()
            )
        );
    }
    Ok(())
}
pub fn rollback(root: &Path, backend: &mut impl Services) -> Result<()> {
    let Some(mut j) = journal(root)? else {
        return Ok(());
    };
    if j.phase == "rolled_back" {
        crate::settings_update::ownership_restored(root)?;
        return Ok(());
    }
    j.phase = "rolling_back".into();
    save(root, &j)?;
    // Do not restart other clock writers if the native service cannot stop.
    backend.change(service::NAME, false)?;
    if j.clock_started {
        backend.neutral()?;
    }
    atomic_write(&root.join("settings.json"), &j.previous_config)?;
    backend.configure(
        service::NAME,
        Some(j.own_service.start_type),
        Some(&j.own_service.account),
    )?;
    if !j.privilege_preexisting {
        backend.privilege(Some(false))?;
    }
    let mut errors = vec![];
    for s in &j.controllers {
        if s.exists
            && let Err(e) = backend
                .configure(&s.name, Some(s.start_type), None)
                .and_then(|_| backend.change(&s.name, s.running))
        {
            errors.push(format!("{}: {e:#}", s.name));
        }
    }
    ensure!(
        errors.is_empty(),
        "Restoration incomplete; retry Restore previous clocks: {}",
        errors.join("; ")
    );
    backend.change(service::NAME, j.own_service.running)?;
    j.phase = "rolled_back".into();
    save(root, &j)?;
    Store::open(&root.join("data/history.sqlite3"), false)?
        .event("takeover_rolled_back", &json!({"restored":j.controllers}))?;
    crate::settings_update::ownership_restored(root)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    struct Fake {
        states: BTreeMap<String, Snapshot>,
        privilege: bool,
        changes: usize,
        fail_at: Option<usize>,
    }
    impl Fake {
        fn new() -> Self {
            let mut states = BTreeMap::new();
            for (name, start, running) in [
                (service::NAME, 3, true),
                ("NetTimeSvc", 2, true),
                ("W32Time", 3, false),
                ("NetTimeNext", 2, true),
            ] {
                states.insert(
                    name.into(),
                    Snapshot {
                        name: name.into(),
                        exists: true,
                        start_type: start,
                        account: "NT AUTHORITY\\LocalService".into(),
                        running,
                    },
                );
            }
            Self {
                states,
                privilege: false,
                changes: 0,
                fail_at: None,
            }
        }
        fn mutation(&mut self) -> Result<()> {
            let index = self.changes;
            self.changes += 1;
            if self.fail_at == Some(index) {
                self.fail_at = None;
                anyhow::bail!("Injected administrative failure at {index}")
            }
            Ok(())
        }
    }
    impl Services for Fake {
        fn snapshot(&mut self, name: &str) -> Result<Snapshot> {
            Ok(self.states[name].clone())
        }
        fn change(&mut self, name: &str, running: bool) -> Result<()> {
            self.mutation()?;
            self.states.get_mut(name).unwrap().running = running;
            Ok(())
        }
        fn configure(
            &mut self,
            name: &str,
            start: Option<u32>,
            account: Option<&str>,
        ) -> Result<()> {
            self.mutation()?;
            let state = self.states.get_mut(name).unwrap();
            if let Some(start) = start {
                state.start_type = start
            }
            if let Some(account) = account {
                state.account = account.into()
            }
            Ok(())
        }
        fn privilege(&mut self, grant: Option<bool>) -> Result<bool> {
            let had = self.privilege;
            if let Some(value) = grant {
                self.mutation()?;
                self.privilege = value;
            }
            Ok(had)
        }
        fn neutral(&mut self) -> Result<()> {
            self.mutation()
        }
    }
    struct Fixture {
        root: std::path::PathBuf,
        config: Vec<u8>,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::current_dir().unwrap().join("work").join(format!(
                "ownership-{}-{}",
                std::process::id(),
                (utc() * 1e9) as u128
            ));
            std::fs::create_dir_all(root.join("data")).unwrap();
            let mut cfg = Config::default();
            cfg.servers.push(crate::config::Server {
                host: "ntp.invalid".into(),
                source_group: "fixture".into(),
                ..Default::default()
            });
            let mut config = vec![0xef, 0xbb, 0xbf];
            config.extend(serde_json::to_vec_pretty(&cfg).unwrap());
            config.extend(b"\r\n");
            std::fs::write(root.join("settings.json"), &config).unwrap();
            let mut store = Store::open(&root.join("data/history.sqlite3"), false).unwrap();
            std::fs::write(
                root.join("data/service-instance.json"),
                br#"{"id":"unit-test-instance"}"#,
            )
            .unwrap();
            let proof = json!({"utc":utc(),"collector_instance":"unit-test-instance","selected":"test-only","synchronization":{"ready":true,"sample_utc":utc(),"offset_ms":1.,"explanation":"Administrative transaction unit fixture; never an actual NTP measurement"}});
            store.save(1, &[], &proof, &[]).unwrap();
            drop(store);
            Self { root, config }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            for name in [
                "settings.json",
                "clock-ownership.json",
                "data/history.sqlite3",
                "data/service-instance.json",
            ] {
                let _ = std::fs::remove_file(self.root.join(name));
            }
            let _ = std::fs::remove_dir(self.root.join("data"));
            let _ = std::fs::remove_dir(&self.root);
        }
    }
    fn assert_restored(fixture: &Fixture, backend: &Fake, original: &BTreeMap<String, Snapshot>) {
        assert_eq!(
            std::fs::read(fixture.root.join("settings.json")).unwrap(),
            fixture.config
        );
        assert_eq!(
            serde_json::to_value(&backend.states).unwrap(),
            serde_json::to_value(original).unwrap()
        );
        assert!(!backend.privilege);
        assert_eq!(
            journal(&fixture.root).unwrap().unwrap().phase,
            "rolled_back"
        );
    }
    #[test]
    fn every_activation_mutation_failure_restores_exact_states_and_config() {
        for fail_at in 0..10 {
            let fixture = Fixture::new();
            let mut backend = Fake::new();
            let original = backend.states.clone();
            backend.fail_at = Some(fail_at);
            assert!(
                activate(&fixture.root, &mut backend).is_err(),
                "failure index {fail_at}"
            );
            assert_restored(&fixture, &backend, &original);
        }
    }
    #[test]
    fn rollback_is_retryable_and_preserves_existing_privilege() {
        let fixture = Fixture::new();
        let mut backend = Fake::new();
        let original = backend.states.clone();
        activate(&fixture.root, &mut backend).unwrap();
        assert!(guard(&fixture.root, &mut backend).unwrap());
        backend.fail_at = Some(backend.changes + 1);
        assert!(rollback(&fixture.root, &mut backend).is_err());
        assert_eq!(
            journal(&fixture.root).unwrap().unwrap().phase,
            "rolling_back"
        );
        assert!(!guard(&fixture.root, &mut backend).unwrap());
        rollback(&fixture.root, &mut backend).unwrap();
        assert_restored(&fixture, &backend, &original);
        rollback(&fixture.root, &mut backend).unwrap();
        backend.privilege = true;
        activate(&fixture.root, &mut backend).unwrap();
        rollback(&fixture.root, &mut backend).unwrap();
        assert!(backend.privilege);
    }
    #[test]
    fn bad_evidence_never_changes_services() {
        let fixture = Fixture::new();
        let mut backend = Fake::new();
        std::fs::write(
            fixture.root.join("data/service-instance.json"),
            br#"{"id":"different-process"}"#,
        )
        .unwrap();
        assert!(activate(&fixture.root, &mut backend).is_err());
        assert_eq!(backend.changes, 0);
        assert!(journal(&fixture.root).unwrap().is_none());
        let mut cfg = Config::default();
        let mut d =
            json!({"utc":1000.,"synchronization":{"ready":true,"sample_utc":1000.,"offset_ms":1.}});
        assert!(readiness(&cfg, &d, 1000.).is_ok());
        assert!(readiness(&cfg, &d, 1200.).is_err());
        d["synchronization"]["offset_ms"] = json!(129.);
        assert!(readiness(&cfg, &d, 1000.).is_ok());
        d["synchronization"]["offset_ms"] = json!(10_001.);
        assert!(readiness(&cfg, &d, 1000.).is_err());
        d["synchronization"]["offset_ms"] = json!(1.);
        d["synchronization"]["sample_utc"] = json!(1001.);
        assert!(readiness(&cfg, &d, 1000.).is_err());
        cfg.clock.max_offset_ms = 0.5;
        cfg.clock.max_recovery_offset_ms = 0.5;
        d["synchronization"]["sample_utc"] = json!(1000.);
        assert!(readiness(&cfg, &d, 1000.).is_err());
    }
}
