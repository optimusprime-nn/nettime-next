//! Bounded slew controller; construction is restricted to the authorized SCM host.
use crate::{
    config::Clock,
    selection::Assessment,
    windows::{check, enable_clock_privilege},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use windows_sys::Win32::System::SystemInformation::{
    GetSystemTimeAdjustmentPrecise, SetSystemTimeAdjustmentPrecise,
};

pub fn adjustment_for_rate(increment: u64, ppm: f64) -> Result<u64> {
    ensure!(
        increment > 0 && ppm.is_finite() && ppm.abs() <= 500.,
        "Invalid clock frequency/rate"
    );
    let frequency = increment as f64 / (1. + ppm / 1_000_000.);
    Ok(if ppm >= 0. {
        frequency.ceil()
    } else {
        frequency.floor()
    } as u64)
}
pub fn rate_for_adjustment(increment: u64, adjustment: u64) -> Result<f64> {
    ensure!(
        increment > 0 && adjustment > 0,
        "Invalid Windows clock frequency"
    );
    Ok((increment as f64 / adjustment as f64 - 1.) * 1_000_000.)
}
pub trait Backend {
    fn rate(&mut self, ppm: f64) -> Result<f64>;
    fn restore(&mut self) -> Result<()>;
}
pub struct WindowsClock {
    increment: u64,
}
impl WindowsClock {
    pub fn new() -> Result<Self> {
        enable_clock_privilege()?;
        let (_, increment, _) = Self::read()?;
        ensure!(increment > 0, "Windows reported zero clock frequency");
        Ok(Self { increment })
    }
    pub fn read() -> Result<(u64, u64, bool)> {
        let (mut actual, mut nominal, mut disabled) = (0, 0, 0);
        check(unsafe { GetSystemTimeAdjustmentPrecise(&mut actual, &mut nominal, &mut disabled) })?;
        Ok((actual, nominal, disabled != 0))
    }
}
impl Backend for WindowsClock {
    fn rate(&mut self, ppm: f64) -> Result<f64> {
        let adjustment = adjustment_for_rate(self.increment, ppm)?;
        check(unsafe { SetSystemTimeAdjustmentPrecise(adjustment, 0) })?;
        let (actual, nominal, disabled) = Self::read()?;
        ensure!(!disabled, "Windows did not enable the clock adjustment");
        let readback = rate_for_adjustment(nominal, actual)?;
        ensure!(
            (readback - ppm).abs() <= 1.,
            "Windows clock adjustment readback differs from request"
        );
        Ok(readback)
    }
    fn restore(&mut self) -> Result<()> {
        check(unsafe { SetSystemTimeAdjustmentPrecise(0, 1) })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub action: String,
    pub rate_ppm: f64,
    pub reason: String,
    pub frequency_ppm: f64,
    pub offset_ms: Option<f64>,
    pub windows_reported_rate_ppm: Option<f64>,
    pub recovering: bool,
}
impl Plan {
    fn new(action: &str, rate: f64, reason: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            rate_ppm: rate,
            reason: reason.into(),
            frequency_ppm: 0.,
            offset_ms: None,
            windows_reported_rate_ppm: None,
            recovering: false,
        }
    }
    fn evidence(mut self, offset: f64, recovering: bool) -> Self {
        self.offset_ms = offset.is_finite().then_some(offset);
        self.recovering = recovering;
        self
    }
}
pub struct Planner {
    options: Clock,
    last_sample: Option<f64>,
    last_offset: f64,
    frequency: f64,
    rate: f64,
    updates: usize,
    recovering: bool,
    pub last_good: Option<f64>,
}
impl Planner {
    pub fn new(options: Clock) -> Self {
        Self {
            options,
            last_sample: None,
            last_offset: 0.,
            frequency: 0.,
            rate: 0.,
            updates: 0,
            recovering: false,
            last_good: None,
        }
    }
    pub fn reset(&mut self) {
        *self = Self::new(self.options.clone());
    }
    pub fn plan(&mut self, assessment: &Assessment, now: f64) -> Plan {
        let offset = assessment.offset_ms.unwrap_or(f64::NAN);
        let stamp = assessment.sample_mono;
        let recovering = offset.abs() > self.options.max_offset_ms;
        let reason = if !assessment.ready || !offset.is_finite() {
            Some(assessment.explanation.as_str())
        } else if offset.abs() > self.options.max_recovery_offset_ms {
            Some("Offset exceeds automatic recovery limit; manual review required; no clock step")
        } else if !stamp.is_finite()
            || now - stamp > self.options.holdover_seconds
            || stamp > now + 1.
        {
            Some("Filtered sample expired; correction suspended")
        } else {
            None
        };
        if let Some(reason) = reason {
            self.reset();
            return Plan::new("suspend", 0., reason).evidence(offset, false);
        }
        // Large errors require a fresh confirmation sequence, even when the
        // ordinary tracking loop was already warm. Reversals restart it too.
        if recovering && (!self.recovering || offset.signum() != self.last_offset.signum()) {
            self.reset();
        }
        self.recovering = recovering;
        if let Some(last) = self.last_sample {
            if stamp <= last {
                return Plan::new(
                    "hold",
                    self.rate,
                    if recovering {
                        "Gradual recovery continues; waiting for a newer system-peer filter sample"
                    } else {
                        "No newer system-peer filter sample; not reused"
                    },
                )
                .evidence(offset, recovering);
            }
            let elapsed = stamp - last;
            if elapsed > self.options.holdover_seconds {
                self.reset();
                self.recovering = recovering;
            } else {
                let estimate = (offset - self.last_offset) * 1000. / elapsed + self.rate;
                self.frequency += 0.125
                    * (estimate.clamp(-self.options.max_slew_ppm, self.options.max_slew_ppm)
                        - self.frequency);
            }
        }
        self.last_sample = Some(stamp);
        self.last_offset = offset;
        self.last_good = Some(now);
        self.updates += 1;
        if self.updates < 3 {
            self.rate = 0.;
            return Plan::new(
                "warmup",
                0.,
                format!(
                    "{}: {}/3 new system-peer samples",
                    if recovering {
                        "Large-offset recovery confirming agreement"
                    } else {
                        "Clock filter settling"
                    },
                    self.updates
                ),
            )
            .evidence(offset, recovering);
        }
        self.rate = (self.frequency + offset * 1000. / 1024.)
            .clamp(-self.options.max_slew_ppm, self.options.max_slew_ppm);
        let mut result = Plan::new(
            "slew",
            self.rate,
            if recovering {
                "Recovering large offset gradually at the configured rate limit; no clock step"
            } else {
                "Bounded gradual correction; no clock step"
            },
        )
        .evidence(offset, recovering);
        result.frequency_ppm = self.frequency;
        result.offset_ms = Some(offset);
        result
    }
}
pub struct Controller<B: Backend> {
    pub planner: Planner,
    backend: B,
    suspended: bool,
}
impl<B: Backend> Controller<B> {
    pub fn new(options: Clock, mut backend: B) -> Result<Self> {
        backend.restore()?;
        Ok(Self {
            planner: Planner::new(options),
            backend,
            suspended: true,
        })
    }
    pub fn update(&mut self, assessment: &Assessment, now: f64, ownership: bool) -> Result<Plan> {
        if !ownership {
            self.close()?;
            anyhow::bail!("Clock ownership conflict; correction stopped");
        }
        let mut plan = self.planner.plan(assessment, now);
        match plan.action.as_str() {
            "suspend" | "warmup" => {
                self.backend.restore()?;
                self.suspended = true;
            }
            "slew" => {
                plan.windows_reported_rate_ppm = Some(self.backend.rate(plan.rate_ppm)?);
                self.suspended = false;
            }
            _ => {}
        }
        Ok(plan)
    }
    pub fn watchdog(&mut self, now: f64, ownership: bool) -> Result<bool> {
        if !ownership {
            self.close()?;
            anyhow::bail!("Clock ownership conflict detected by watchdog");
        }
        if !self.suspended
            && self
                .planner
                .last_good
                .is_some_and(|good| now - good > self.planner.options.holdover_seconds)
        {
            self.close()?;
            return Ok(true);
        }
        Ok(false)
    }
    pub fn close(&mut self) -> Result<()> {
        self.backend.restore()?;
        self.planner.reset();
        self.suspended = true;
        Ok(())
    }
}
impl<B: Backend> Drop for Controller<B> {
    fn drop(&mut self) {
        let _ = self.backend.restore();
    }
}

/// The watchdog runs independently of DNS/TLS/UDP waits in the polling thread.
pub struct Session {
    controller: std::sync::Arc<std::sync::Mutex<Controller<WindowsClock>>>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    failure: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    worker: Option<std::thread::JoinHandle<()>>,
    root: std::path::PathBuf,
}
impl Session {
    pub fn start(root: &std::path::Path, options: Clock) -> Result<Self> {
        ensure!(
            crate::ownership::guard(root, &mut crate::ownership::WindowsServices)?,
            "Clock ownership is not authorized or another controller is running"
        );
        let controller = std::sync::Arc::new(std::sync::Mutex::new(Controller::new(
            options,
            WindowsClock::new()?,
        )?));
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let failure = std::sync::Arc::new(std::sync::Mutex::new(None));
        let c = controller.clone();
        let stop = cancelled.clone();
        let error = failure.clone();
        let folder = root.to_path_buf();
        let worker = std::thread::spawn(move || {
            use std::sync::atomic::Ordering;
            while !stop.load(Ordering::Relaxed) {
                let result = (|| -> Result<bool> {
                    let guard =
                        crate::ownership::guard(&folder, &mut crate::ownership::WindowsServices)
                            .unwrap_or(false);
                    c.lock()
                        .map_err(|_| anyhow::anyhow!("Clock controller lock failed"))?
                        .watchdog(crate::protocol::mono(), guard)
                })();
                match result {
                    Err(e) => {
                        if let Ok(mut lock) = c.lock() {
                            let _ = lock.close();
                        }
                        if let Ok(mut value) = error.lock() {
                            *value = Some(format!("{e:#}"));
                        }
                        break;
                    }
                    Ok(true) => {
                        let _ = crate::storage::Store::open(
                            &folder.join("data/history.sqlite3"),
                            false,
                        )
                        .and_then(|s| {
                            s.event(
                                "clock_holdover_expired",
                                &serde_json::json!({"action":"Restored neutral clock rate"}),
                            )
                        });
                    }
                    _ => {}
                }
                for _ in 0..20 {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        });
        Ok(Self {
            controller,
            cancelled,
            failure,
            worker: Some(worker),
            root: root.to_path_buf(),
        })
    }
    pub fn check(&self) -> Result<()> {
        if let Some(error) = self
            .failure
            .lock()
            .map_err(|_| anyhow::anyhow!("Clock watchdog lock failed"))?
            .as_ref()
        {
            anyhow::bail!("{error}");
        }
        Ok(())
    }
    pub fn update(&self, assessment: &Assessment, now: f64) -> Result<Plan> {
        self.check()?;
        let guard = crate::ownership::guard(&self.root, &mut crate::ownership::WindowsServices)
            .unwrap_or(false);
        self.controller
            .lock()
            .map_err(|_| anyhow::anyhow!("Clock controller lock failed"))?
            .update(assessment, now, guard)
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Ok(mut c) = self.controller.lock() {
            let _ = c.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};
    #[test]
    fn frequency_direction_and_quantization_are_correct() {
        for rate in [-500., -200., -0.01, 0., 0.01, 200., 500.] {
            let value = adjustment_for_rate(10_000_000, rate).unwrap();
            let actual = rate_for_adjustment(10_000_000, value).unwrap();
            assert!(actual.abs() <= rate.abs() + 0.000001);
            assert!((actual - rate).abs() < 0.11);
        }
        assert!(adjustment_for_rate(0, 1.).is_err());
        assert!(adjustment_for_rate(100, f64::NAN).is_err());
        assert!(adjustment_for_rate(100, 501.).is_err());
    }
    struct Fake(Rc<RefCell<Vec<f64>>>);
    impl Backend for Fake {
        fn rate(&mut self, ppm: f64) -> Result<f64> {
            self.0.borrow_mut().push(ppm);
            Ok(ppm)
        }
        fn restore(&mut self) -> Result<()> {
            self.0.borrow_mut().push(0.);
            Ok(())
        }
    }
    fn assessment(stamp: f64, offset: f64) -> Assessment {
        Assessment {
            ready: true,
            sample_mono: stamp,
            offset_ms: Some(offset),
            ..Default::default()
        }
    }
    #[test]
    fn controller_warms_deduplicates_bounds_and_restores_on_conflict() {
        let calls = Rc::new(RefCell::new(vec![]));
        let mut c = Controller::new(Clock::default(), Fake(calls.clone())).unwrap();
        for stamp in [1., 65.] {
            assert_eq!(
                c.update(&assessment(stamp, 100.), stamp, true)
                    .unwrap()
                    .action,
                "warmup"
            );
        }
        let plan = c.update(&assessment(129., 100.), 129., true).unwrap();
        assert_eq!(plan.action, "slew");
        assert!(plan.rate_ppm > 0. && plan.rate_ppm <= 250.);
        let count = calls.borrow().len();
        assert_eq!(
            c.update(&assessment(129., 100.), 130., true)
                .unwrap()
                .action,
            "hold"
        );
        assert_eq!(calls.borrow().len(), count);
        assert!(c.watchdog(131., false).is_err());
        assert_eq!(*calls.borrow().last().unwrap(), 0.);
    }
    #[test]
    fn stale_excessive_leap_and_holdover_suspend() {
        let mut p = Planner::new(Clock::default());
        assert_eq!(p.plan(&assessment(1., 10_001.), 1.).action, "suspend");
        assert_eq!(p.plan(&assessment(1., 1.), 902.).action, "suspend");
        assert_eq!(p.plan(&assessment(10., 1.), 1.).action, "suspend");
        let mut a = assessment(1., 1.);
        a.ready = false;
        a.explanation = "Leap announcement".into();
        assert_eq!(p.plan(&a, 1.).action, "suspend");
        let calls = Rc::new(RefCell::new(vec![]));
        let mut c = Controller::new(Clock::default(), Fake(calls.clone())).unwrap();
        for stamp in [1., 65., 129.] {
            c.update(&assessment(stamp, -5.), stamp, true).unwrap();
        }
        assert!(c.watchdog(1030., true).unwrap());
        assert_eq!(*calls.borrow().last().unwrap(), 0.);
    }
    #[test]
    fn closed_loop_converges_for_both_oscillator_directions() {
        for drift in [-35., 35.] {
            let mut planner = Planner::new(Clock::default());
            let mut offset = 20.;
            let mut rate = 0.;
            for step in 0..1000 {
                offset -= (drift + rate) * 64. / 1000.;
                let stamp = 1. + step as f64 * 64.;
                let plan = planner.plan(&assessment(stamp, offset), stamp);
                assert_ne!(plan.action, "suspend");
                assert!(plan.rate_ppm.abs() <= 250.);
                rate = plan.rate_ppm;
            }
            assert!(
                offset.abs() < 0.1,
                "drift={drift}, remaining offset={offset}"
            );
            assert!((rate + drift).abs() < 0.1);
        }
    }
    #[test]
    fn recovery_confirms_new_samples_and_stops_on_lost_agreement() {
        let calls = Rc::new(RefCell::new(vec![]));
        let mut c = Controller::new(Clock::default(), Fake(calls.clone())).unwrap();
        for stamp in [1., 65.] {
            let plan = c.update(&assessment(stamp, -2_000.), stamp, true).unwrap();
            assert_eq!(plan.action, "warmup");
            assert!(plan.recovering);
            assert_eq!(*calls.borrow().last().unwrap(), 0.);
            // Repeated assessments do not advance confirmation or apply a rate.
            assert_eq!(
                c.update(&assessment(stamp, -2_000.), stamp + 1., true)
                    .unwrap()
                    .action,
                "hold"
            );
        }
        let plan = c.update(&assessment(129., -2_000.), 129., true).unwrap();
        assert_eq!(plan.action, "slew");
        assert!(plan.recovering && plan.rate_ppm == -250.);
        assert_eq!(plan.windows_reported_rate_ppm, Some(-250.));
        let bad = Assessment {
            ready: false,
            ..assessment(193., -2_000.)
        };
        assert_eq!(c.update(&bad, 193., true).unwrap().action, "suspend");
        assert_eq!(*calls.borrow().last().unwrap(), 0.);
        assert_eq!(
            c.update(&assessment(257., -2_000.), 257., true)
                .unwrap()
                .action,
            "warmup"
        );
        // A sign reversal requires confirmation from scratch.
        assert_eq!(
            c.update(&assessment(321., 2_000.), 321., true)
                .unwrap()
                .action,
            "warmup"
        );
        assert_eq!(c.planner.updates, 1);
        assert!(c.update(&assessment(385., 2_000.), 385., false).is_err());
        assert_eq!(*calls.borrow().last().unwrap(), 0.);
    }
    #[test]
    fn large_offsets_recover_in_both_directions_and_survive_an_outage() {
        for initial in [-2_000., 2_000.] {
            for drift in [-35., 35.] {
                let mut planner = Planner::new(Clock::default());
                let mut offset = initial;
                let mut rate = 0.;
                let mut recovered = false;
                for step in 0..1600 {
                    offset -= (drift + rate) * 64. / 1000.;
                    let stamp = 1. + step as f64 * 64.;
                    let mut a = assessment(stamp, offset);
                    // An outage during recovery must pause, then reconfirm.
                    if (10..30).contains(&step) {
                        a.ready = false;
                    }
                    let plan = planner.plan(&a, stamp);
                    if !a.ready {
                        assert_eq!(plan.action, "suspend");
                    } else {
                        assert_ne!(plan.action, "suspend");
                    }
                    assert!(plan.rate_ppm.abs() <= 250.);
                    recovered |= plan.action == "slew" && plan.recovering;
                    rate = plan.rate_ppm;
                }
                assert!(recovered);
                assert!(
                    offset.abs() < 0.1,
                    "initial={initial}, drift={drift}, remaining={offset}"
                );
                assert!((rate + drift).abs() < 0.1);
            }
        }
    }
    #[test]
    fn recovery_rejects_stale_evidence_and_can_be_disabled_by_its_limit() {
        let mut p = Planner::new(Clock::default());
        let plan = p.plan(&assessment(1., -2_000.), 902.);
        assert_eq!(plan.action, "suspend");
        assert_eq!(plan.offset_ms, Some(-2_000.));
        let mut p = Planner::new(Clock {
            max_recovery_offset_ms: 128.,
            ..Clock::default()
        });
        assert_eq!(p.plan(&assessment(1., 129.), 1.).action, "suspend");
        let mut p = Planner::new(Clock::default());
        let mut a = assessment(1., 2_000.);
        a.offset_ms = Some(f64::NAN);
        assert_eq!(p.plan(&a, 1.).action, "suspend");
    }
    #[test]
    fn leaving_normal_tracking_requires_fresh_recovery_confirmation() {
        let mut p = Planner::new(Clock::default());
        for stamp in [1., 65., 129.] {
            p.plan(&assessment(stamp, -20.), stamp);
        }
        let first = p.plan(&assessment(193., -2_000.), 193.);
        assert_eq!(first.action, "warmup");
        assert_eq!(first.rate_ppm, 0.);
        assert!(first.recovering);
        assert_eq!(p.plan(&assessment(257., -2_000.), 257.).action, "warmup");
        assert_eq!(p.plan(&assessment(321., -2_000.), 321.).action, "slew");
        // Returning inside the tracking threshold retains the learned rate loop.
        let normal = p.plan(&assessment(385., -100.), 385.);
        assert_eq!(normal.action, "slew");
        assert!(!normal.recovering);
    }
}
