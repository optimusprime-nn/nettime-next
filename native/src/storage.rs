use crate::protocol::{Sample, utc};
use anyhow::{Result, ensure};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::Value;
use std::io::{BufWriter, Read, Write};
use std::{path::Path, time::Duration};
pub struct Store {
    pub db: Connection,
    logging_enabled: bool,
    diagnostic_logging: bool,
}
fn payload_text(value: rusqlite::types::ValueRef<'_>) -> rusqlite::Result<String> {
    use rusqlite::types::ValueRef;
    match value {
        ValueRef::Text(bytes) => String::from_utf8(bytes.to_vec())
            .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e))),
        ValueRef::Blob(bytes) => {
            let mut text = String::new();
            GzDecoder::new(bytes)
                .take(16 * 1024 * 1024 + 1)
                .read_to_string(&mut text)
                .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))?;
            if text.len() > 16 * 1024 * 1024 {
                return Err(rusqlite::Error::InvalidQuery);
            }
            Ok(text)
        }
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
fn compact_decision(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("servers"); // Actual measurements already have their own rows.
    }
    if let Some(sync) = value["synchronization"].as_object_mut()
        && let Some(survivors) = sync.get_mut("survivors")
        && let Some(rows) = survivors.as_array()
    {
        *survivors = Value::Array(rows.iter().map(|s| s["peer"].clone()).collect());
    }
    value
}
impl Store {
    pub fn open(path: &Path, read_only: bool) -> Result<Self> {
        if !read_only && let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let db = Connection::open_with_flags(
            path,
            if read_only {
                OpenFlags::SQLITE_OPEN_READ_ONLY
            } else {
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
            },
        )?;
        db.busy_timeout(Duration::from_secs(10))?;
        db.create_scalar_function(
            "history_json",
            1,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8
                | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
            |ctx| payload_text(ctx.get_raw(0)),
        )?;
        if !read_only {
            db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
        CREATE TABLE IF NOT EXISTS samples(id INTEGER PRIMARY KEY,cycle INTEGER,utc REAL,host TEXT,port INTEGER,status TEXT,stratum INTEGER,offset_ms REAL,delay_ms REAL,payload TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS samples_time ON samples(utc); CREATE INDEX IF NOT EXISTS samples_host ON samples(host,port,utc);
        CREATE TABLE IF NOT EXISTS decisions(id INTEGER PRIMARY KEY,cycle INTEGER,utc REAL,selected TEXT,confidence TEXT,payload TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS decisions_time ON decisions(utc);
        CREATE TABLE IF NOT EXISTS events(id INTEGER PRIMARY KEY,utc REAL,kind TEXT,payload TEXT);
        CREATE TABLE IF NOT EXISTS peer_state(peer TEXT PRIMARY KEY,payload TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS live_state(id INTEGER PRIMARY KEY CHECK(id=1),cycle INTEGER,payload TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS events_time ON events(utc);
        CREATE INDEX IF NOT EXISTS samples_uncompressed ON samples(utc) WHERE typeof(payload)='text';
        CREATE INDEX IF NOT EXISTS decisions_uncompressed ON decisions(utc) WHERE typeof(payload)='text';
        CREATE INDEX IF NOT EXISTS events_uncompressed ON events(utc) WHERE typeof(payload)='text';")?;
        }
        let cfg = path
            .parent()
            .and_then(Path::parent)
            .and_then(|p| crate::config::Config::load(&p.join("settings.json")).ok());
        Ok(Self {
            db,
            logging_enabled: cfg.as_ref().is_none_or(|c| c.logging_enabled),
            diagnostic_logging: cfg.is_some_and(|c| c.diagnostic_logging),
        })
    }
    pub fn configure_logging(&mut self, cfg: &crate::config::Config) {
        self.logging_enabled = cfg.logging_enabled;
        self.diagnostic_logging = cfg.diagnostic_logging;
    }
    pub fn event(&self, kind: &str, value: &Value) -> Result<()> {
        if !self.logging_enabled {
            return Ok(());
        }
        self.db.execute(
            "INSERT INTO events(utc,kind,payload) VALUES(?,?,?)",
            params![utc(), kind, value.to_string()],
        )?;
        Ok(())
    }
    pub fn next_cycle(&self) -> Result<i64> {
        let history: i64 =
            self.db
                .query_row("SELECT COALESCE(MAX(cycle),0)+1 FROM decisions", [], |r| {
                    r.get(0)
                })?;
        let live: i64 =
            self.db
                .query_row("SELECT COALESCE(MAX(cycle),0)+1 FROM live_state", [], |r| {
                    r.get(0)
                })?;
        Ok(history.max(live))
    }
    pub fn latest(&self) -> Result<Option<Value>> {
        let has_live: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='live_state')",
            [],
            |r| r.get(0),
        )?;
        if has_live
            && let Some(raw) = self
                .db
                .query_row("SELECT payload FROM live_state WHERE id=1", [], |r| {
                    r.get::<_, String>(0)
                })
                .optional()?
        {
            return Ok(Some(serde_json::from_str(&raw)?));
        }
        let raw: Option<String> = self
            .db
            .query_row(
                "SELECT history_json(payload) FROM decisions ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(raw.map(|r| serde_json::from_str(&r)).transpose()?)
    }
    pub fn save(
        &mut self,
        cycle: i64,
        samples: &[Sample],
        decision: &Value,
        states: &[(String, Value)],
    ) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO live_state(id,cycle,payload) VALUES(1,?,?)",
            params![cycle, decision.to_string()],
        )?;
        if self.logging_enabled {
            for s in samples {
                let valid = s.status == "valid";
                let mut data = s.data();
                if !self.diagnostic_logging
                    && let Some(object) = data.as_object_mut()
                {
                    object.remove("request_hex");
                    object.remove("reply_hex");
                }
                tx.execute("INSERT INTO samples(cycle,utc,host,port,status,stratum,offset_ms,delay_ms,payload) VALUES(?,?,?,?,?,?,?,?,?)",params![cycle,s.utc,s.host,s.port,s.status,s.stratum,valid.then_some(s.offset_ms),valid.then_some(s.delay_ms),data.to_string()])?;
            }
            tx.execute(
                "INSERT INTO decisions(cycle,utc,selected,confidence,payload) VALUES(?,?,?,?,?)",
                params![
                    cycle,
                    utc(),
                    decision["selected"].as_str(),
                    decision["confidence"].as_str().unwrap_or(""),
                    if self.diagnostic_logging {
                        decision.clone()
                    } else {
                        compact_decision(decision)
                    }
                    .to_string()
                ],
            )?;
        }
        for (key, state) in states {
            tx.execute(
                "INSERT OR REPLACE INTO peer_state(peer,payload) VALUES(?,?)",
                params![key, state.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn states(&self) -> Result<Vec<(String, Value)>> {
        let mut q = self.db.prepare("SELECT peer,payload FROM peer_state")?;
        let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.map(|r| {
            let (k, v) = r?;
            Ok((k, serde_json::from_str(&v)?))
        })
        .collect()
    }
    pub fn prune(&self, days: f64) -> Result<()> {
        self.prune_at(days, utc())
    }
    pub fn prune_at(&self, days: f64, now: f64) -> Result<()> {
        ensure!(
            days.is_finite() && (1. ..=3650.).contains(&days) && now.is_finite(),
            "Invalid retention period"
        );
        let cutoff = now - days * 86400.;
        for table in ["samples", "decisions", "events"] {
            // Short transactions let current measurements continue during cleanup.
            loop {
                let deleted = self.db.execute(&format!("DELETE FROM {table} WHERE id IN (SELECT id FROM {table} WHERE utc<? LIMIT 128)"), [cutoff])?;
                if deleted == 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        Ok(())
    }
    pub fn compress_before(&mut self, cutoff: f64) -> Result<usize> {
        let mut count = 0;
        for table in ["samples", "decisions", "events"] {
            loop {
                let batch = {
                    let mut q = self.db.prepare(&format!("SELECT id,payload FROM {table} WHERE utc<? AND typeof(payload)='text' ORDER BY utc LIMIT 64"))?;
                    q.query_map([cutoff], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                };
                if batch.is_empty() {
                    break;
                }
                // Compress outside the write transaction. Replacement is atomic.
                let packed = batch
                    .into_iter()
                    .map(|(id, text)| -> Result<_> {
                        let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
                        gzip.write_all(text.as_bytes())?;
                        Ok((id, gzip.finish()?))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let tx = self.db.transaction()?;
                for (id, bytes) in packed {
                    count += tx.execute(
                        &format!(
                            "UPDATE {table} SET payload=? WHERE id=? AND typeof(payload)='text'"
                        ),
                        params![bytes, id],
                    )?;
                }
                tx.commit()?;
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        Ok(count)
    }
    pub fn maintain(&mut self, days: f64) -> Result<Value> {
        // A new collector can start before its administrator transaction commits.
        // Never delete history for a settings change that may still roll back.
        if let Some(root) = self
            .db
            .path()
            .map(Path::new)
            .and_then(Path::parent)
            .and_then(Path::parent)
        {
            for journal in ["settings-update.json", "upgrade-state.json"] {
                let path = root.join(journal);
                if path.exists() {
                    let state: Value = serde_json::from_slice(&std::fs::read(path)?)?;
                    if state["phase"] == "prepared" {
                        return Ok(
                            serde_json::json!({"deferred":true,"reason":"Administrator transaction has not committed"}),
                        );
                    }
                }
            }
        }
        self.prune(days)?;
        let compressed = self.compress_before(utc() - 86400.)?;
        let free: i64 = self
            .db
            .query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
        let pages: i64 = self.db.query_row("PRAGMA page_count", [], |r| r.get(0))?;
        let mut reclaimed = false;
        if free > 256 && free * 4 > pages {
            // Limit the time writers can be held up; interrupted VACUUM rolls back.
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            self.db
                .progress_handler(1000, Some(move || std::time::Instant::now() > deadline))?;
            let result = self.db.execute_batch("VACUUM");
            self.db.progress_handler(0, None::<fn() -> bool>)?;
            match result {
                Ok(()) => reclaimed = true,
                Err(rusqlite::Error::SqliteFailure(e, _))
                    if e.code == rusqlite::ErrorCode::OperationInterrupted
                        || e.code == rusqlite::ErrorCode::DatabaseBusy => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(
            serde_json::json!({"compressed_records":compressed,"space_reclaimed":reclaimed,"retention_days":days}),
        )
    }
    pub fn history(&self, host: &str, port: u16) -> Result<Vec<[f64; 2]>> {
        let mut q=self.db.prepare("SELECT utc,offset_ms FROM samples WHERE host=? AND port=? AND status='valid' ORDER BY utc,id")?;
        Ok(
            q.query_map(params![host, port], |r| Ok([r.get(0)?, r.get(1)?]))?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        )
    }
    pub fn export(&self, folder: &Path) -> Result<()> {
        std::fs::create_dir_all(folder)?;
        let snapshot = self.db.unchecked_transaction()?;
        for table in ["samples", "decisions", "events"] {
            let mut q = snapshot.prepare(&format!("SELECT * FROM {table} ORDER BY id"))?;
            let names = q
                .column_names()
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
            let mut rows = q.query([])?;
            let mut output = GzEncoder::new(
                BufWriter::new(std::fs::File::create(
                    folder.join(format!("{table}.json.gz")),
                )?),
                Compression::default(),
            );
            output.write_all(b"[\n")?;
            let mut first = true;
            while let Some(row) = rows.next()? {
                let mut record = serde_json::Map::new();
                for (i, name) in names.iter().enumerate() {
                    use rusqlite::types::ValueRef;
                    let value = match row.get_ref(i)? {
                        ValueRef::Null => Value::Null,
                        ValueRef::Integer(v) => serde_json::json!(v),
                        ValueRef::Real(v) => serde_json::json!(v),
                        ValueRef::Text(v) => {
                            let text = std::str::from_utf8(v)?;
                            if name == "payload" {
                                serde_json::from_str(text)?
                            } else {
                                Value::String(text.into())
                            }
                        }
                        ValueRef::Blob(_) if name == "payload" => {
                            serde_json::from_str(&payload_text(row.get_ref(i)?)?)?
                        }
                        ValueRef::Blob(v) => Value::String(hex::encode(v)),
                    };
                    record.insert(name.clone(), value);
                }
                if !first {
                    output.write_all(b",\n")?;
                }
                first = false;
                serde_json::to_writer(&mut output, &record)?;
            }
            output.write_all(b"\n]\n")?;
            output.finish()?.flush()?;
        }
        snapshot.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn pending_settings_do_not_purge_history_before_administrator_commit() {
        let root = std::env::current_dir().unwrap().join("work").join(format!(
            "pending-retention-{}-{}",
            std::process::id(),
            utc().to_bits()
        ));
        let path = root.join("data/history.sqlite3");
        let journal = root.join("settings-update.json");
        let mut store = Store::open(&path, false).unwrap();
        store
            .db
            .execute(
                "INSERT INTO events(utc,kind,payload)VALUES(?,'proof','{}')",
                [utc() - 6. * 86400.],
            )
            .unwrap();
        std::fs::write(&journal, br#"{"phase":"prepared"}"#).unwrap();
        assert_eq!(store.maintain(5.).unwrap()["deferred"], true);
        let count: i64 = store
            .db
            .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        std::fs::write(&journal, br#"{"phase":"complete"}"#).unwrap();
        store.maintain(5.).unwrap();
        let count: i64 = store
            .db
            .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        drop(store);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(journal).unwrap();
        std::fs::remove_dir(root.join("data")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn logging_off_keeps_live_status_and_restart_cycle_without_growing_history() {
        let mut store = Store::open(Path::new(":memory:"), false).unwrap();
        let cfg = crate::config::Config {
            logging_enabled: false,
            ..Default::default()
        };
        store.configure_logging(&cfg);
        let decision = json!({"utc":1_800_000_000.,"selected":"a:123","servers":{"a:123":{"status":"valid"}},"clock_action":{"action":"slew","rate_ppm":12.}});
        store
            .save(
                50,
                &[Sample::default()],
                &decision,
                &[("a:123".into(), json!({"reach":255}))],
            )
            .unwrap();
        store
            .event("clock_correction", &json!({"action":"slew"}))
            .unwrap();
        for table in ["samples", "decisions", "events"] {
            let count: i64 = store
                .db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0);
        }
        assert_eq!(store.latest().unwrap(), Some(decision));
        assert_eq!(store.next_cycle().unwrap(), 51);
        assert_eq!(store.states().unwrap()[0].1["reach"], 255);
    }
    #[test]
    fn compression_is_lossless_and_retention_deletes_text_and_compressed_records_at_boundary() {
        let mut store = Store::open(Path::new(":memory:"), false).unwrap();
        let now = 1_800_000_000.;
        let keep = now - 5. * 86400.;
        for table in ["samples", "decisions", "events"] {
            for stamp in [keep - 1., keep, now - 86400. - 1., now - 86400. + 1.] {
                store
                    .db
                    .execute(
                        &format!("INSERT INTO {table}(utc,payload)VALUES(?,?)"),
                        params![
                            stamp,
                            json!({"proof":"lossless round trip","stamp":stamp}).to_string()
                        ],
                    )
                    .unwrap();
            }
        }
        assert_eq!(store.compress_before(now - 86400.).unwrap(), 9);
        assert_eq!(store.compress_before(now - 86400.).unwrap(), 0);
        for table in ["samples", "decisions", "events"] {
            let raw: String = store
                .db
                .query_row(
                    &format!("SELECT history_json(payload) FROM {table} WHERE utc=?"),
                    [keep],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&raw).unwrap(),
                json!({"proof":"lossless round trip","stamp":keep})
            );
            let raw_type: String = store
                .db
                .query_row(
                    &format!("SELECT typeof(payload) FROM {table} WHERE utc=?"),
                    [now - 86400. + 1.],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(raw_type, "text");
        }
        store.prune_at(5., now).unwrap();
        for table in ["samples", "decisions", "events"] {
            let count: i64 = store
                .db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 3);
            let oldest: f64 = store
                .db
                .query_row(&format!("SELECT min(utc) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(oldest, keep);
        }
        assert!(store.prune_at(0., now).is_err());
    }
    #[test]
    fn compact_history_keeps_decision_evidence_and_compressed_reports_match() {
        let mut store = Store::open(Path::new(":memory:"), false).unwrap();
        let now = utc().floor();
        let sample = Sample {
            host: "a".into(),
            port: 123,
            utc: now - 90000.,
            status: "valid".into(),
            stratum: 1,
            offset_ms: 2.,
            delay_ms: 3.,
            authentication_status: "authenticated".into(),
            request_hex: "packet".into(),
            ..Default::default()
        };
        let decision = json!({"utc":now-90000.,"selected":"a:123","servers":{"a:123":sample.data()},"synchronization":{"ready":true,"explanation":"selected by distance","reasons":{"a:123":"System peer; weight 1"},"interval_seconds":64.,"candidates":[{"peer":"a:123","distance_ms":3.}],"survivors":[{"peer":"a:123","distance_ms":3.}]}});
        store.save(1, &[sample], &decision, &[]).unwrap();
        let raw: String = store
            .db
            .query_row("SELECT payload FROM decisions", [], |r| r.get(0))
            .unwrap();
        let compact: Value = serde_json::from_str(&raw).unwrap();
        assert!(compact.get("servers").is_none());
        assert_eq!(compact["synchronization"]["survivors"], json!(["a:123"]));
        assert!(raw.len() < decision.to_string().len());
        let headers: Option<String> = store
            .db
            .query_row(
                "SELECT json_extract(payload,'$.request_hex') FROM samples",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(headers.is_none());
        // Storage uses the persisted record timestamp, not the clock of this test.
        store
            .db
            .execute("UPDATE decisions SET utc=?", [now - 90000.])
            .unwrap();
        let before = crate::comparison::build(&store, 7., now).unwrap();
        store.compress_before(now - 86400.).unwrap();
        let after = crate::comparison::build(&store, 7., now).unwrap();
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(after).unwrap()
        );
        assert_eq!(store.latest().unwrap(), Some(decision));
        let corrupt = payload_text(rusqlite::types::ValueRef::Blob(b"not gzip"));
        assert!(corrupt.is_err());
    }
    #[test]
    fn failed_measurements_do_not_bias_offsets_and_exports_keep_event_timing() {
        let folder = std::env::current_dir().unwrap().join("work").join(format!(
            "native-store-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let database = folder.join("history.sqlite3");
        let mut store = Store::open(&database, false).unwrap();
        let sample = Sample {
            host: "invalid.example".into(),
            port: 123,
            status: "error".into(),
            ..Default::default()
        };
        store
            .save(
                1,
                &[sample],
                &serde_json::json!({"selected":null,"confidence":"None"}),
                &[],
            )
            .unwrap();
        let offset: Option<f64> = store
            .db
            .query_row("SELECT offset_ms FROM samples", [], |r| r.get(0))
            .unwrap();
        assert!(offset.is_none());
        store
            .event("proof", &serde_json::json!({"reason":"example"}))
            .unwrap();
        store.export(&folder).unwrap();
        let events: Value = serde_json::from_reader(GzDecoder::new(
            std::fs::File::open(folder.join("events.json.gz")).unwrap(),
        ))
        .unwrap();
        assert_eq!(events[0]["kind"], "proof");
        assert!(events[0]["utc"].as_f64().unwrap() > 0.);
        assert_eq!(events[0]["payload"]["reason"], "example");
        drop(store);
        for file in [
            "history.sqlite3",
            "samples.json.gz",
            "decisions.json.gz",
            "events.json.gz",
        ] {
            std::fs::remove_file(folder.join(file)).unwrap();
        }
        std::fs::remove_dir(folder).unwrap();
    }
}
