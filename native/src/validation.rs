//! Physical actuator verification, hard-restricted to disposable hosted CI runners.
use crate::{
    clock::{Backend, WindowsClock},
    ownership::{CONTROLLERS, Services, WindowsServices},
    protocol::{mono, utc},
    service,
};
use anyhow::{Result, ensure};
use serde_json::json;
use std::{path::Path, time::Duration};
fn response() -> Result<f64> {
    let start = mono();
    let mut readings = vec![];
    while mono() - start < 20. {
        let before = mono();
        let wall = utc();
        let after = mono();
        if after - before < 0.001 {
            readings.push(((before + after) / 2., wall));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    ensure!(
        readings.len() >= 100,
        "Insufficient precise clock observations"
    );
    let (m0, w0) = readings[0];
    let points: Vec<_> = readings
        .iter()
        .map(|(m, w)| (m - m0, (w - w0) - (m - m0)))
        .collect();
    let mx = points.iter().map(|p| p.0).sum::<f64>() / points.len() as f64;
    let my = points.iter().map(|p| p.1).sum::<f64>() / points.len() as f64;
    Ok(points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum::<f64>()
        / points.iter().map(|p| (p.0 - mx).powi(2)).sum::<f64>()
        * 1e6)
}
pub fn clock_response(output: &Path) -> Result<()> {
    ensure!(
        std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            && std::env::var("RUNNER_ENVIRONMENT").as_deref() == Ok("github-hosted"),
        "Physical clock-response verification is restricted to disposable GitHub-hosted runners"
    );
    crate::windows::require_admin()?;
    for name in CONTROLLERS.into_iter().chain([service::NAME]) {
        ensure!(
            !WindowsServices.snapshot(name)?.running,
            "Clock service {name} must be stopped"
        );
    }
    let mut clock = WindowsClock::new()?;
    let mut records = vec![];
    let result = (|| -> Result<()> {
        for target in [0., 200., -200.] {
            let readback = clock.rate(target)?;
            std::thread::sleep(Duration::from_secs(1));
            let observed = response()?;
            let record = json!({"target_wall_rate_ppm":target,"readback_wall_rate_ppm":readback,"observed_wall_vs_qpc_ppm":observed});
            println!("{record}");
            records.push(record);
        }
        let baseline = records[0]["observed_wall_vs_qpc_ppm"].as_f64().unwrap();
        for record in &mut records[1..] {
            let effective = record["observed_wall_vs_qpc_ppm"].as_f64().unwrap() - baseline;
            record["observed_relative_to_baseline_ppm"] = json!(effective);
            ensure!(
                (effective - record["target_wall_rate_ppm"].as_f64().unwrap()).abs() < 30.,
                "Incorrect physical clock direction/gain: {record}"
            );
        }
        Ok(())
    })();
    let restored = clock.restore();
    std::fs::write(
        output,
        serde_json::to_vec_pretty(
            &json!({"passed":result.is_ok()&&restored.is_ok(),"records":records}),
        )?,
    )?;
    restored?;
    result
}
