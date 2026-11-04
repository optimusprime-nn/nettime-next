//! Clock controller: sets the clock when independent sources agree it is off by
//! more than the tracking limit, and otherwise slews it at a bounded rate.
//! Construction is restricted to the authorized SCM host.
use crate::{
    config::Clock,
    selection::Assessment,
    windows::{check, enable_clock_privilege},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use windows_sys::Win32::{
    Foundation::{FILETIME, SYSTEMTIME},
    System::{
        SystemInformation::{
            GetSystemTimeAdjustmentPrecise, GetSystemTimePreciseAsFileTime, SetSystemTime,
            SetSystemTimeAdjustmentPrecise,
        },
        Time::FileTimeToSystemTime,
    },
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
    /// Moves the clock by `offset_ms` at once.
    fn step(&mut self, offset_ms: f64) -> Result<()>;
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
    /// Setting the time is also the only documented way to update the PC's
    /// hardware clock, which Windows reads at the next boot.
    fn step(&mut self, offset_ms: f64) -> Result<()> {
        ensure!(offset_ms.is_finite(), "Invalid clock step");
        let mut now = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        unsafe { GetSystemTimePreciseAsFileTime(&mut now) };
        // FILETIME counts 100 ns; SYSTEMTIME holds milliseconds, so add half a
        // millisecond for the truncation to round instead.
        let ticks = ((now.dwHighDateTime as i64) << 32 | now.dwLowDateTime as i64)
            + (offset_ms * 10_000.).round() as i64
            + 5_000;
        ensure!(ticks > 0, "Clock step out of range");
        let target = FILETIME {
            dwLowDateTime: ticks as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        };
        let mut time: SYSTEMTIME = unsafe { std::mem::zeroed() };
        check(unsafe { FileTimeToSystemTime(&target, &mut time) })?;
        check(unsafe { SetSystemTime(&time) })
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
        }
    }
    fn evidence(mut self, offset: f64) -> Self {
        self.offset_ms = offset.is_finite().then_some(offset);
        self
    }
}
pub struct Planner {
    options: Clock,
    last_sample: Option<f64>,
    last_offset: f64,
    frequency: f64,
    /// Sample time and offset the next frequency estimate is measured from.
    /// It only moves when an estimate is made, so a newer sample from a source
    /// polled in the same round (milliseconds later) cannot turn a small
    /// offset difference between servers into a huge apparent drift.
    anchor: Option<(f64, f64)>,
    rate: f64,
    /// When each rate took effect (monotonic seconds), kept back to the
    /// anchor, so the frequency estimate uses the rates actually applied
    /// between the two sample times rather than only the current one.
    rate_log: Vec<(f64, f64)>,
    updates: usize,
    /// Sample time of the assessment that last set the clock. Only samples
    /// measured after it can be trusted; this survives `reset`.
    stepped: Option<f64>,
    pub last_good: Option<f64>,
}
impl Planner {
    pub fn new(options: Clock) -> Self {
        Self {
            options,
            last_sample: None,
            last_offset: 0.,
            frequency: 0.,
            anchor: None,
            rate: 0.,
            rate_log: vec![],
            updates: 0,
            stepped: None,
            last_good: None,
        }
    }
    pub fn reset(&mut self) {
        let stepped = self.stepped;
        *self = Self::new(self.options.clone());
        self.stepped = stepped;
    }
    fn set_rate(&mut self, rate: f64, now: f64) {
        self.rate = rate;
        self.rate_log.push((now, rate));
    }
    /// Mean applied rate over [from, to]; zero before the first logged rate,
    /// as the clock starts and resumes from the neutral rate.
    fn mean_rate(&self, from: f64, to: f64) -> f64 {
        if to <= from {
            return self.rate;
        }
        let (mut total, mut time, mut rate) = (0., from, 0.);
        for &(at, next) in &self.rate_log {
            if at <= from {
                rate = next;
                continue;
            }
            if at >= to {
                break;
            }
            total += rate * (at - time);
            time = at;
            rate = next;
        }
        total += rate * (to - time);
        total / (to - from)
    }
    pub fn plan(&mut self, assessment: &Assessment, now: f64) -> Plan {
        let offset = assessment.offset_ms.unwrap_or(f64::NAN);
        let stamp = assessment.sample_mono;
        let reason = if !assessment.ready || !offset.is_finite() {
            Some(assessment.explanation.as_str())
        } else if !stamp.is_finite()
            || now - stamp > self.options.holdover_seconds
            || stamp > now + 1.
        {
            Some("Filtered sample expired; correction suspended")
        } else if self.stepped.is_some_and(|s| stamp <= s) {
            Some("Waiting for a sample measured after the clock was set")
        } else {
            None
        };
        if let Some(reason) = reason {
            self.reset();
            return Plan::new("suspend", 0., reason).evidence(offset);
        }
        // Independent sources agree on the time, so set the clock to it now
        // rather than slewing a large error away over hours.
        if offset.abs() > self.options.max_offset_ms {
            self.reset();
            self.stepped = Some(stamp);
            return Plan::new("step", 0., "Sources agree; clock set to the agreed time")
                .evidence(offset);
        }
        if let Some(last) = self.last_sample {
            if stamp <= last {
                return Plan::new(
                    "hold",
                    self.rate,
                    "No newer system-peer filter sample; not reused",
                )
                .evidence(offset);
            }
            if stamp - last > self.options.holdover_seconds {
                self.reset();
            }
        }
        // Estimate frequency only over at least half a poll interval (16 s
        // minimum); a shorter span leaves the anchor in place for later.
        let span = (assessment.interval_seconds / 2.).max(16.);
        match self.anchor {
            Some((from, _)) if stamp - from > self.options.holdover_seconds => {
                self.anchor = Some((stamp, offset));
            }
            Some((from, anchor_offset)) if stamp - from >= span => {
                let estimate =
                    (offset - anchor_offset) * 1000. / (stamp - from) + self.mean_rate(from, stamp);
                self.frequency += 0.125
                    * (estimate.clamp(-self.options.max_slew_ppm, self.options.max_slew_ppm)
                        - self.frequency);
                self.anchor = Some((stamp, offset));
            }
            Some(_) => {}
            None => self.anchor = Some((stamp, offset)),
        }
        self.last_sample = Some(stamp);
        self.last_offset = offset;
        // Keep the rate in force at the anchor time and everything after it.
        let from = self.anchor.map_or(stamp, |a| a.0);
        if let Some(keep) = self.rate_log.iter().rposition(|&(at, _)| at <= from) {
            self.rate_log.drain(..keep);
        }
        self.last_good = Some(now);
        self.updates += 1;
        if self.updates < 3 {
            self.set_rate(0., now);
            return Plan::new(
                "warmup",
                0.,
                format!(
                    "Clock filter settling: {}/3 new system-peer samples",
                    self.updates
                ),
            )
            .evidence(offset);
        }
        self.set_rate(
            (self.frequency + offset * 1000. / 1024.)
                .clamp(-self.options.max_slew_ppm, self.options.max_slew_ppm),
            now,
        );
        let mut result =
            Plan::new("slew", self.rate, "Bounded gradual correction").evidence(offset);
        result.frequency_ppm = self.frequency;
        result.offset_ms = Some(offset);
        result
    }
}
/// Outcome of an ownership check. A check that fails (an SCM query error, or
/// another service briefly in a pending state) is Unknown, not a conflict:
/// correction pauses at the neutral rate until ownership is verified again,
/// instead of stopping the service for good.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ownership {
    Verified,
    Conflict,
    Unknown,
}
impl Ownership {
    pub fn check(root: &std::path::Path) -> Self {
        match crate::ownership::guard(root, &mut crate::ownership::WindowsServices) {
            Ok(true) => Self::Verified,
            Ok(false) => Self::Conflict,
            Err(_) => Self::Unknown,
        }
    }
}
impl From<bool> for Ownership {
    fn from(verified: bool) -> Self {
        if verified {
            Self::Verified
        } else {
            Self::Conflict
        }
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
    pub fn update(
        &mut self,
        assessment: &Assessment,
        now: f64,
        ownership: impl Into<Ownership>,
    ) -> Result<Plan> {
        match ownership.into() {
            Ownership::Conflict => {
                self.close()?;
                anyhow::bail!("Clock ownership conflict; correction stopped");
            }
            Ownership::Unknown => {
                self.close()?;
                return Ok(Plan::new(
                    "suspend",
                    0.,
                    "Clock ownership could not be verified; correction paused at the neutral rate",
                ));
            }
            Ownership::Verified => {}
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
            "step" => {
                self.backend.restore()?;
                self.backend.step(plan.offset_ms.unwrap_or(f64::NAN))?;
                self.suspended = true;
            }
            _ => {}
        }
        Ok(plan)
    }
    pub fn watchdog(&mut self, now: f64, ownership: impl Into<Ownership>) -> Result<bool> {
        match ownership.into() {
            Ownership::Conflict => {
                self.close()?;
                anyhow::bail!("Clock ownership conflict detected by watchdog");
            }
            Ownership::Unknown => {
                if !self.suspended {
                    self.close()?;
                }
                return Ok(false);
            }
            Ownership::Verified => {}
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
                    let guard = Ownership::check(&folder);
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
        let guard = Ownership::check(&self.root);
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
    /// Records applied rates, and clock steps separately.
    struct Fake(Rc<RefCell<Vec<f64>>>, Rc<RefCell<Vec<f64>>>);
    impl Backend for Fake {
        fn rate(&mut self, ppm: f64) -> Result<f64> {
            self.0.borrow_mut().push(ppm);
            Ok(ppm)
        }
        fn restore(&mut self) -> Result<()> {
            self.0.borrow_mut().push(0.);
            Ok(())
        }
        fn step(&mut self, offset_ms: f64) -> Result<()> {
            self.1.borrow_mut().push(offset_ms);
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
        let mut c =
            Controller::new(Clock::default(), Fake(calls.clone(), Default::default())).unwrap();
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
    fn unverifiable_ownership_pauses_without_stopping() {
        let calls = Rc::new(RefCell::new(vec![]));
        let mut c =
            Controller::new(Clock::default(), Fake(calls.clone(), Default::default())).unwrap();
        for stamp in [1., 65., 129.] {
            c.update(&assessment(stamp, 100.), stamp, true).unwrap();
        }
        assert!(*calls.borrow().last().unwrap() > 0.);
        assert!(!c.watchdog(130., Ownership::Unknown).unwrap());
        assert_eq!(*calls.borrow().last().unwrap(), 0.);
        let plan = c
            .update(&assessment(193., 100.), 193., Ownership::Unknown)
            .unwrap();
        assert_eq!(plan.action, "suspend");
        assert_eq!(plan.rate_ppm, 0.);
        // Once verified again, correction restarts through warmup.
        let plan = c.update(&assessment(257., 100.), 257., true).unwrap();
        assert_eq!(plan.action, "warmup");
        assert!(c.watchdog(258., Ownership::Conflict).is_err());
    }
    #[test]
    fn frequency_estimate_tracks_a_drifting_clock_with_old_samples() {
        // A local clock 20 ppm slow, measured through filter samples that are
        // 400 s old when they are used, as at a long holdover with poll 64 s.
        const DRIFT: f64 = 20.;
        let mut p = Planner::new(Clock::default());
        let mut applied: Vec<(f64, f64)> = vec![(0., 0.)];
        let offset_at = |applied: &[(f64, f64)], t: f64| {
            let mut total = 0.;
            for (i, &(from, rate)) in applied.iter().enumerate() {
                let to = applied.get(i + 1).map_or(t, |next| next.0).min(t);
                if to > from {
                    total += (DRIFT - rate) * (to - from) / 1000.;
                }
            }
            total
        };
        let mut worst_late = 0_f64;
        for k in 1..=120 {
            let stamp = 64. * k as f64;
            let now = stamp + 400.;
            let plan = p.plan(&assessment(stamp, offset_at(&applied, stamp)), now);
            applied.push((now, plan.rate_ppm));
            if k > 60 {
                worst_late = worst_late.max((p.frequency - DRIFT).abs());
            }
        }
        assert!(worst_late < 1., "frequency error {worst_late:.3} ppm");
    }
    #[test]
    fn system_peer_switch_within_one_poll_round_does_not_kick_frequency() {
        // As observed on 1.0.1: all sources are polled at once, so when the
        // system peer switches between two of them the new sample is only
        // milliseconds newer, with an offset that differs by the servers'
        // own disagreement. The local clock is 28 ppm fast.
        const DRIFT: f64 = -28.;
        let mut p = Planner::new(Clock::default());
        let mut applied: Vec<(f64, f64)> = vec![(0., 0.)];
        let offset_at = |applied: &[(f64, f64)], t: f64| {
            let mut total = 0.;
            for (i, &(from, rate)) in applied.iter().enumerate() {
                let to = applied.get(i + 1).map_or(t, |next| next.0).min(t);
                if to > from {
                    total += (DRIFT - rate) * (to - from) / 1000.;
                }
            }
            total
        };
        let (mut worst_frequency, mut worst_offset) = (0_f64, 0_f64);
        for k in 1..=300 {
            let stamp = 64. * k as f64;
            let offset = offset_at(&applied, stamp);
            let plan = p.plan(&assessment(stamp, offset), stamp + 1.);
            applied.push((stamp + 1., plan.rate_ppm));
            if k % 3 == 0 {
                // Another server from the same round becomes system peer.
                let plan = p.plan(&assessment(stamp + 0.005, offset - 0.4), stamp + 2.);
                applied.push((stamp + 2., plan.rate_ppm));
            }
            if k > 100 {
                worst_frequency = worst_frequency.max((p.frequency - DRIFT).abs());
                worst_offset = worst_offset.max(offset.abs());
            }
        }
        assert!(
            worst_frequency < 3.,
            "frequency error {worst_frequency:.2} ppm"
        );
        assert!(worst_offset < 2., "offset reached {worst_offset:.2} ms");
    }
    #[test]
    fn stale_leap_and_holdover_suspend() {
        let mut p = Planner::new(Clock::default());
        assert_eq!(p.plan(&assessment(1., 1.), 902.).action, "suspend");
        assert_eq!(p.plan(&assessment(10., 1.), 1.).action, "suspend");
        let mut a = assessment(1., 1.);
        a.ready = false;
        a.explanation = "Leap announcement".into();
        assert_eq!(p.plan(&a, 1.).action, "suspend");
        let calls = Rc::new(RefCell::new(vec![]));
        let mut c =
            Controller::new(Clock::default(), Fake(calls.clone(), Default::default())).unwrap();
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
    fn agreed_large_offset_sets_the_clock_at_once_in_either_direction() {
        // Seconds-scale errors (as after a boot from a drifted hardware clock)
        // and absurd ones alike are set straight to the agreed time.
        for offset in [-7_800., 2_000., -129., 86_400_000.] {
            let (rates, steps) = (Rc::default(), Rc::default());
            let mut c =
                Controller::new(Clock::default(), Fake(Rc::clone(&rates), Rc::clone(&steps)))
                    .unwrap();
            let plan = c.update(&assessment(1., offset), 1., true).unwrap();
            assert_eq!(plan.action, "step", "{offset}");
            assert_eq!(plan.offset_ms, Some(offset));
            assert_eq!(*steps.borrow(), [offset]);
            // The rate is neutral when the clock is set.
            assert_eq!(*rates.borrow().last().unwrap(), 0.);
        }
    }
    #[test]
    fn offsets_within_the_tracking_limit_are_slewed_not_stepped() {
        let (rates, steps) = (Rc::default(), Rc::default());
        let mut c =
            Controller::new(Clock::default(), Fake(Rc::clone(&rates), Rc::clone(&steps))).unwrap();
        for stamp in [1., 65., 129.] {
            c.update(&assessment(stamp, -127.), stamp, true).unwrap();
        }
        assert!(steps.borrow().is_empty());
        assert!(*rates.borrow().last().unwrap() < 0.);
    }
    #[test]
    fn the_clock_is_never_set_without_fresh_agreement() {
        let steps: Rc<RefCell<Vec<f64>>> = Rc::default();
        let mut c =
            Controller::new(Clock::default(), Fake(Rc::default(), Rc::clone(&steps))).unwrap();
        let no_agreement = Assessment {
            ready: false,
            ..assessment(1., -7_800.)
        };
        assert_eq!(c.update(&no_agreement, 1., true).unwrap().action, "suspend");
        // Stale evidence.
        assert_eq!(
            c.update(&assessment(1., -7_800.), 902., true)
                .unwrap()
                .action,
            "suspend"
        );
        let mut unknown = assessment(1., 0.);
        unknown.offset_ms = Some(f64::NAN);
        assert_eq!(c.update(&unknown, 1., true).unwrap().action, "suspend");
        // Unverified or conflicting ownership.
        assert_eq!(
            c.update(&assessment(2., -7_800.), 2., Ownership::Unknown)
                .unwrap()
                .action,
            "suspend"
        );
        assert!(c.update(&assessment(3., -7_800.), 3., false).is_err());
        assert!(steps.borrow().is_empty());
    }
    #[test]
    fn evidence_from_before_a_step_cannot_set_the_clock_again() {
        let steps: Rc<RefCell<Vec<f64>>> = Rc::default();
        let mut c =
            Controller::new(Clock::default(), Fake(Rc::default(), Rc::clone(&steps))).unwrap();
        assert_eq!(
            c.update(&assessment(10., -7_800.), 10., true)
                .unwrap()
                .action,
            "step"
        );
        // The same or an older filter sample, still showing the old offset.
        for stamp in [10., 9.] {
            let plan = c.update(&assessment(stamp, -7_800.), 11., true).unwrap();
            assert_eq!(plan.action, "suspend");
        }
        assert_eq!(steps.borrow().len(), 1);
        // Samples measured after the step resume normal tracking.
        for stamp in [74., 138.] {
            assert_eq!(
                c.update(&assessment(stamp, 0.4), stamp, true)
                    .unwrap()
                    .action,
                "warmup"
            );
        }
        assert_eq!(
            c.update(&assessment(202., 0.4), 202., true).unwrap().action,
            "slew"
        );
        assert_eq!(steps.borrow().len(), 1);
    }
}
