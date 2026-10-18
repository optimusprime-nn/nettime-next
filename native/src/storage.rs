use crate::protocol::{Sample, utc};
use anyhow::Result;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::Value;
use std::io::{BufWriter, Write};
use std::{path::Path, time::Duration};
pub struct Store {
    pub db: Connection,
}
impl Store {
    pub fn open(path: &Path, read_only: bool) -> Result<Self> {
        if !read_only && let Some(parent) = path.parent() {
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
        if !read_only {
            db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
        CREATE TABLE IF NOT EXISTS samples(id INTEGER PRIMARY KEY,cycle INTEGER,utc REAL,host TEXT,port INTEGER,status TEXT,stratum INTEGER,offset_ms REAL,delay_ms REAL,payload TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS samples_time ON samples(utc); CREATE INDEX IF NOT EXISTS samples_host ON samples(host,port,utc);
        CREATE TABLE IF NOT EXISTS decisions(id INTEGER PRIMARY KEY,cycle INTEGER,utc REAL,selected TEXT,confidence TEXT,payload TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS decisions_time ON decisions(utc);
        CREATE TABLE IF NOT EXISTS events(id INTEGER PRIMARY KEY,utc REAL,kind TEXT,payload TEXT);
        CREATE TABLE IF NOT EXISTS peer_state(peer TEXT PRIMARY KEY,payload TEXT NOT NULL);")?;
        }
        Ok(Self { db })
    }
    pub fn event(&self, kind: &str, value: &Value) -> Result<()> {
        self.db.execute(
            "INSERT INTO events(utc,kind,payload) VALUES(?,?,?)",
            params![utc(), kind, value.to_string()],
        )?;
        Ok(())
    }
    pub fn next_cycle(&self) -> Result<i64> {
        Ok(self
            .db
            .query_row("SELECT COALESCE(MAX(cycle),0)+1 FROM decisions", [], |r| {
                r.get(0)
            })?)
    }
    pub fn latest(&self) -> Result<Option<Value>> {
        let raw: Option<String> = self
            .db
            .query_row(
                "SELECT payload FROM decisions ORDER BY id DESC LIMIT 1",
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
        for s in samples {
            let valid = s.status == "valid";
            tx.execute("INSERT INTO samples(cycle,utc,host,port,status,stratum,offset_ms,delay_ms,payload) VALUES(?,?,?,?,?,?,?,?,?)",params![cycle,s.utc,s.host,s.port,s.status,s.stratum,valid.then_some(s.offset_ms),valid.then_some(s.delay_ms),s.data().to_string()])?;
        }
        tx.execute(
            "INSERT INTO decisions(cycle,utc,selected,confidence,payload) VALUES(?,?,?,?,?)",
            params![
                cycle,
                utc(),
                decision["selected"].as_str(),
                decision["confidence"].as_str().unwrap_or(""),
                decision.to_string()
            ],
        )?;
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
        let cutoff = utc() - days.max(7.) * 86400.;
        for table in ["samples", "decisions", "events"] {
            self.db
                .execute(&format!("DELETE FROM {table} WHERE utc<?"), [cutoff])?;
        }
        Ok(())
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
            let mut output =
                BufWriter::new(std::fs::File::create(folder.join(format!("{table}.json")))?);
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
            output.flush()?;
        }
        snapshot.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let events: Value =
            serde_json::from_str(&std::fs::read_to_string(folder.join("events.json")).unwrap())
                .unwrap();
        assert_eq!(events[0]["kind"], "proof");
        assert!(events[0]["utc"].as_f64().unwrap() > 0.);
        assert_eq!(events[0]["payload"]["reason"], "example");
        drop(store);
        for file in [
            "history.sqlite3",
            "samples.json",
            "decisions.json",
            "events.json",
        ] {
            std::fs::remove_file(folder.join(file)).unwrap();
        }
        std::fs::remove_dir(folder).unwrap();
    }
}
