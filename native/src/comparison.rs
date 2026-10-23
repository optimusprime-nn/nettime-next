//! Windowed reports from a single SQLite snapshot, including selection carry-in.
use crate::storage::Store;
use anyhow::{Result, ensure};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufWriter, Write},
    path::Path,
};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Source {
    pub peer: String,
    pub attempts: usize,
    pub valid: usize,
    pub valid_percent: Option<f64>,
    pub first_utc: Option<f64>,
    pub last_utc: Option<f64>,
    pub observed_hours: f64,
    pub median_offset_ms: Option<f64>,
    pub p95_abs_offset_ms: Option<f64>,
    pub median_rtt_ms: Option<f64>,
    pub p95_rtt_ms: Option<f64>,
    pub offset_stddev_ms: Option<f64>,
    pub min_stratum: Option<i64>,
    pub max_stratum: Option<i64>,
    pub stratum_changes: usize,
    pub authenticated_valid: usize,
    pub authenticated_percent: Option<f64>,
    pub selected_hours: f64,
    pub selection_entries: usize,
    pub longest_attempt_gap_seconds: f64,
    pub failures: BTreeMap<String, usize>,
    pub selection_reasons: BTreeMap<String, usize>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub utc: f64,
    pub selected: Option<String>,
    pub previous: Option<String>,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub start_utc: f64,
    pub end_utc: f64,
    pub requested_days: f64,
    pub servers: Vec<Source>,
    pub selection_changes: Vec<Change>,
    pub notes: Vec<String>,
    #[serde(skip)]
    pub offset_series: BTreeMap<String, Vec<[f64; 2]>>,
}
type Reading = (f64, String, Option<i64>, Option<f64>, Option<f64>, String);
fn percentile(values: &[f64], fraction: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    let index = (values.len() - 1) as f64 * fraction;
    let lo = index.floor() as usize;
    let hi = index.ceil() as usize;
    Some(values[lo] + (values[hi] - values[lo]) * (index - lo as f64))
}
struct Decision {
    stamp: f64,
    selected: Option<String>,
    reason: String,
    reasons: BTreeMap<String, String>,
    interval: f64,
}
fn compact(stamp: f64, raw: &str) -> Result<Decision> {
    let v: Value = serde_json::from_str(raw)?;
    let selected = v["selected"].as_str().map(String::from);
    let sync = &v["synchronization"];
    let reasons = sync
        .get("reasons")
        .or_else(|| v.get("reasons"))
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.into())))
                .collect()
        })
        .unwrap_or_default();
    let interval = sync["interval_seconds"]
        .as_f64()
        .or_else(|| {
            v["servers"]
                .as_array()
                .and_then(|s| s.iter().find(|s| s["peer"].as_str() == selected.as_deref()))
                .and_then(|s| s["interval_seconds"].as_f64())
        })
        .filter(|v| v.is_finite() && *v >= 64. && *v <= 131072.)
        .unwrap_or(64.);
    Ok(Decision {
        stamp,
        selected,
        reason: sync["explanation"]
            .as_str()
            .or_else(|| v["explanation"].as_str())
            .unwrap_or("")
            .into(),
        reasons,
        interval,
    })
}
pub fn build(store: &Store, days: f64, now: f64) -> Result<Report> {
    ensure!(
        days.is_finite() && (0. ..=3650.).contains(&days) && days > 0. && now.is_finite(),
        "Invalid report range"
    );
    let start = now - days * 86400.;
    let snapshot = store.db.unchecked_transaction()?;
    let mut grouped: BTreeMap<String, Vec<Reading>> = BTreeMap::new();
    {
        let mut q=snapshot.prepare("SELECT utc,host,port,status,stratum,offset_ms,delay_ms,json_extract(history_json(payload),'$.authentication_status') FROM samples WHERE utc>=? AND utc<=? ORDER BY utc,id")?;
        let mut rows = q.query(params![start, now])?;
        while let Some(r) = rows.next()? {
            let key = format!("{}:{}", r.get::<_, String>(1)?, r.get::<_, u16>(2)?);
            grouped.entry(key).or_default().push((
                r.get(0)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get::<_, Option<String>>(7)?.unwrap_or_default(),
            ));
        }
    }
    let mut decisions = vec![];
    {
        let mut q = snapshot.prepare(
            "SELECT utc,history_json(payload) FROM decisions WHERE utc<? ORDER BY utc DESC,id DESC LIMIT 1",
        )?;
        let mut rows = q.query([start])?;
        if let Some(r) = rows.next()? {
            decisions.push(compact(r.get(0)?, &r.get::<_, String>(1)?)?);
        }
        let mut q = snapshot
            .prepare("SELECT utc,history_json(payload) FROM decisions WHERE utc>=? AND utc<=? ORDER BY utc,id")?;
        let mut rows = q.query(params![start, now])?;
        while let Some(r) = rows.next()? {
            decisions.push(compact(r.get(0)?, &r.get::<_, String>(1)?)?);
        }
    }
    snapshot.commit()?;
    let mut spans: BTreeMap<String, f64> = BTreeMap::new();
    let mut entries: BTreeMap<String, usize> = BTreeMap::new();
    let mut reasons: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut changes = vec![];
    let mut known: BTreeSet<String> = grouped.keys().cloned().collect();
    for (i, d) in decisions.iter().enumerate() {
        let end = now
            .min(decisions.get(i + 1).map_or(now, |n| n.stamp))
            .min(d.stamp + 3. * d.interval);
        if let Some(peer) = &d.selected {
            known.insert(peer.clone());
            *spans.entry(peer.clone()).or_default() += (end - d.stamp.max(start)).max(0.);
        }
        if d.stamp < start {
            continue;
        }
        let previous = i.checked_sub(1).and_then(|i| decisions[i].selected.clone());
        if d.selected != previous {
            if let Some(peer) = &d.selected {
                *entries.entry(peer.clone()).or_default() += 1;
            }
            changes.push(Change {
                utc: d.stamp,
                selected: d.selected.clone(),
                previous,
                reason: d.reason.clone(),
            });
        }
        for (peer, reason) in &d.reasons {
            known.insert(peer.clone());
            let reason = reason
                .split(';')
                .next()
                .unwrap_or(reason)
                .split(" (")
                .next()
                .unwrap_or(reason);
            *reasons
                .entry(peer.clone())
                .or_default()
                .entry(reason.into())
                .or_default() += 1;
        }
    }
    let mut servers = vec![];
    let mut offset_series = BTreeMap::new();
    for peer in known {
        let rows = grouped.remove(&peer).unwrap_or_default();
        let valid: Vec<_> = rows
            .iter()
            .filter(|r| {
                r.1 == "valid" && r.3.is_some_and(f64::is_finite) && r.4.is_some_and(f64::is_finite)
            })
            .collect();
        let offsets: Vec<f64> = valid.iter().filter_map(|r| r.3).collect();
        let delays: Vec<f64> = valid.iter().filter_map(|r| r.4).collect();
        let strata: Vec<i64> = valid.iter().filter_map(|r| r.2).collect();
        let mean = if offsets.is_empty() {
            0.
        } else {
            offsets.iter().sum::<f64>() / offsets.len() as f64
        };
        let authenticated = valid.iter().filter(|r| r.5 == "authenticated").count();
        let mut failures = BTreeMap::new();
        for row in &rows {
            if row.1 != "valid" {
                *failures.entry(row.1.clone()).or_default() += 1;
            }
        }
        let first = rows.first().map(|r| r.0);
        let last = rows.last().map(|r| r.0);
        offset_series.insert(
            peer.clone(),
            valid.iter().map(|r| [r.0, r.3.unwrap()]).collect(),
        );
        servers.push(Source {
            peer: peer.clone(),
            attempts: rows.len(),
            valid: valid.len(),
            valid_percent: (!rows.is_empty())
                .then(|| 100. * valid.len() as f64 / rows.len() as f64),
            first_utc: first,
            last_utc: last,
            observed_hours: (last.unwrap_or(start) - first.unwrap_or(start)) / 3600.,
            median_offset_ms: percentile(&offsets, 0.5),
            p95_abs_offset_ms: percentile(
                &offsets.iter().map(|v| v.abs()).collect::<Vec<_>>(),
                0.95,
            ),
            median_rtt_ms: percentile(&delays, 0.5),
            p95_rtt_ms: percentile(&delays, 0.95),
            offset_stddev_ms: (!offsets.is_empty()).then(|| {
                (offsets.iter().map(|o| (o - mean).powi(2)).sum::<f64>() / offsets.len() as f64)
                    .sqrt()
            }),
            min_stratum: strata.iter().copied().min(),
            max_stratum: strata.iter().copied().max(),
            stratum_changes: strata.windows(2).filter(|s| s[0] != s[1]).count(),
            authenticated_valid: authenticated,
            authenticated_percent: (!valid.is_empty())
                .then(|| 100. * authenticated as f64 / valid.len() as f64),
            selected_hours: spans.get(&peer).copied().unwrap_or(0.) / 3600.,
            selection_entries: entries.get(&peer).copied().unwrap_or(0),
            longest_attempt_gap_seconds: rows
                .windows(2)
                .map(|s| s[1].0 - s[0].0)
                .fold(0., f64::max),
            failures,
            selection_reasons: reasons.remove(&peer).unwrap_or_default(),
        });
    }
    Ok(Report {
        start_utc: start,
        end_utc: now,
        requested_days: days,
        servers,
        selection_changes: changes,
        offset_series,
        notes: vec![
            "Valid percentage describes recorded attempts, not continuous uptime.".into(),
            "Coverage includes gaps; missing time is never backfilled.".into(),
            "Offset variation includes local PC drift and corrections, not physical PPS jitter."
                .into(),
            "Selection durations expire after three poll intervals; downtime is not credited."
                .into(),
        ],
    })
}
fn csv_cell(value: &str) -> String {
    // Prevent spreadsheet formula interpretation of user-controlled server/reason fields.
    let prefix = if value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        "'"
    } else {
        ""
    };
    format!("\"{prefix}{}\"", value.replace('"', "\"\""))
}
fn csv_records(path: &Path, records: &[Value], fallback: &[&str]) -> Result<()> {
    let mut file = BufWriter::new(std::fs::File::create(path)?);
    file.write_all(&[0xef, 0xbb, 0xbf])?;
    let fields: Vec<String> = records
        .first()
        .and_then(Value::as_object)
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_else(|| fallback.iter().map(|v| v.to_string()).collect());
    writeln!(
        file,
        "{}",
        fields
            .iter()
            .map(|s| csv_cell(s))
            .collect::<Vec<_>>()
            .join(",")
    )?;
    for row in records {
        let cells = fields
            .iter()
            .map(|key| match &row[key] {
                Value::Null => String::new(),
                Value::String(s) => csv_cell(s),
                Value::Number(n) => n.to_string(),
                v => csv_cell(&v.to_string()),
            })
            .collect::<Vec<_>>();
        writeln!(file, "{}", cells.join(","))?;
    }
    file.flush()?;
    Ok(())
}
pub fn export(report: &Report, folder: &Path) -> Result<()> {
    std::fs::create_dir_all(folder)?;
    serde_json::to_writer_pretty(
        BufWriter::new(std::fs::File::create(folder.join("comparison.json"))?),
        report,
    )?;
    csv_records(
        &folder.join("comparison.csv"),
        &report
            .servers
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()?,
        &["peer", "attempts", "valid"],
    )?;
    csv_records(
        &folder.join("selection-changes.csv"),
        &report
            .selection_changes
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()?,
        &["utc", "selected", "previous", "reason"],
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windowed_carry_in_expiration_statistics_and_future_exclusion() {
        let store = Store::open(Path::new(":memory:"), false).unwrap();
        store
            .db
            .execute_batch("DROP TABLE samples;DROP TABLE decisions;")
            .unwrap();
        store.db.execute_batch("CREATE TABLE samples(id INTEGER PRIMARY KEY,utc REAL,host TEXT,port INTEGER,status TEXT,stratum INTEGER,offset_ms REAL,delay_ms REAL,payload TEXT);CREATE TABLE decisions(id INTEGER PRIMARY KEY,utc REAL,payload TEXT);").unwrap();
        for (stamp, offset, stratum, status) in [
            (100., -10., 1, "valid"),
            (200., 30., 2, "valid"),
            (300., 0., 0, "timeout"),
            (2000., 9999., 1, "valid"),
        ] {
            store.db.execute("INSERT INTO samples(utc,host,port,status,stratum,offset_ms,delay_ms,payload)VALUES(?,'a',123,?,?,?,?,?)",params![stamp,status,stratum,offset,2.,r#"{"authentication_status":"authenticated"}"#]).unwrap();
        }
        for (stamp, selected) in [
            (0., Some("a:123")),
            (500., None),
            (2000., Some("future:123")),
        ] {
            let payload = serde_json::json!({"selected":selected,"synchronization":{"interval_seconds":64,"reasons":{"a:123":"System peer; weight 1"},"explanation":"proof"}});
            store
                .db
                .execute(
                    "INSERT INTO decisions(utc,payload)VALUES(?,?)",
                    params![stamp, payload.to_string()],
                )
                .unwrap();
        }
        let r = build(&store, 1000. / 86400., 1100.).unwrap();
        let s = &r.servers[0];
        assert_eq!(r.servers.len(), 1);
        assert_eq!(s.attempts, 3);
        assert_eq!(s.valid, 2);
        assert_eq!(s.median_offset_ms, Some(10.));
        assert_eq!(s.offset_stddev_ms, Some(20.));
        assert_eq!(s.stratum_changes, 1);
        assert!((s.selected_hours * 3600. - 92.).abs() < 0.001);
        assert_eq!(s.selection_entries, 0);
        assert_eq!(r.selection_changes.len(), 1);
        assert_eq!(s.authenticated_percent, Some(100.));
    }
    #[test]
    fn csv_strings_are_quoted_and_formula_safe() {
        assert_eq!(csv_cell("=1+2"), "\"'=1+2\"");
        assert_eq!(csv_cell("a,\"b"), "\"a,\"\"b\"");
    }
}
