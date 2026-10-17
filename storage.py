"""Transactional rolling evidence store, with measurements and decision exports."""
import csv
import json
import sqlite3
import time
from contextlib import contextmanager
from pathlib import Path


class Store:
    def __init__(self, path, retention_days=7):
        self.path = Path(path)
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.retention_days = max(7, retention_days)
        with self.connect() as db:
            db.execute("PRAGMA journal_mode=WAL")
            db.executescript('''
                CREATE TABLE IF NOT EXISTS samples (
                  id INTEGER PRIMARY KEY, cycle INTEGER, utc REAL, host TEXT,
                  port INTEGER, status TEXT, stratum INTEGER, offset_ms REAL,
                  delay_ms REAL, payload TEXT NOT NULL);
                CREATE INDEX IF NOT EXISTS samples_time ON samples(utc);
                CREATE INDEX IF NOT EXISTS samples_host ON samples(host,port,utc);
                CREATE TABLE IF NOT EXISTS decisions (
                  id INTEGER PRIMARY KEY, cycle INTEGER, utc REAL, selected TEXT,
                  confidence TEXT, payload TEXT NOT NULL);
                CREATE INDEX IF NOT EXISTS decisions_time ON decisions(utc);
                CREATE TABLE IF NOT EXISTS events (
                  id INTEGER PRIMARY KEY, utc REAL, kind TEXT, payload TEXT);
                CREATE TABLE IF NOT EXISTS peer_state (
                  peer TEXT PRIMARY KEY, payload TEXT NOT NULL);
            ''')

    @contextmanager
    def connect(self):
        db = sqlite3.connect(self.path, timeout=10)
        try:
            db.execute("PRAGMA synchronous=FULL")
            with db:
                yield db
        finally:
            db.close()

    def event(self, kind, payload, utc=None):
        with self.connect() as db:
            db.execute("INSERT INTO events(utc,kind,payload) VALUES (?,?,?)",
                       (time.time() if utc is None else utc, kind, json.dumps(payload)))

    def save_cycle(self, cycle, samples, decision, state, utc=None):
        utc = time.time() if utc is None else utc
        with self.connect() as db:
            for s in samples:
                db.execute("""INSERT INTO samples(cycle,utc,host,port,status,stratum,offset_ms,delay_ms,payload)
                            VALUES (?,?,?,?,?,?,?,?,?)""",
                           (cycle, s.utc, s.host, s.port, s.status, s.stratum, s.offset_ms, s.delay_ms,
                            json.dumps(s.data())))
            db.execute("INSERT INTO decisions(cycle,utc,selected,confidence,payload) VALUES (?,?,?,?,?)",
                       (cycle, utc, decision["selected"], decision["confidence"], json.dumps(decision)))
            for key, value in state.items():
                db.execute("INSERT OR REPLACE INTO peer_state(peer,payload) VALUES (?,?)",
                           (key, json.dumps(value)))

    def load_state(self):
        with self.connect() as db:
            return {key: json.loads(value) for key, value in db.execute("SELECT peer,payload FROM peer_state")}

    def next_cycle(self):
        with self.connect() as db:
            return db.execute("SELECT COALESCE(MAX(cycle),0)+1 FROM decisions").fetchone()[0]

    def prune(self, now=None):
        cutoff = (time.time() if now is None else now) - self.retention_days * 86400
        with self.connect() as db:
            for table in ("samples", "decisions", "events"):
                db.execute(f"DELETE FROM {table} WHERE utc < ?", (cutoff,))

    def stats(self):
        with self.connect() as db:
            db.row_factory = sqlite3.Row
            return [dict(row) for row in db.execute('''
              SELECT host,port,COUNT(*) attempts,
                SUM(CASE WHEN status='valid' THEN 1 ELSE 0 END) valid,
                MIN(utc) first_utc,MAX(utc) last_utc,AVG(offset_ms) mean_offset_ms,
                MIN(offset_ms) min_offset_ms,MAX(offset_ms) max_offset_ms,
                AVG(delay_ms) mean_delay_ms,MIN(stratum) min_stratum,MAX(stratum) max_stratum
              FROM samples GROUP BY host,port ORDER BY host,port''')]

    def history(self, host, port):
        with self.connect() as db:
            return db.execute("""SELECT utc,offset_ms,delay_ms,stratum,status FROM samples
                                 WHERE host=? AND port=? ORDER BY utc,id""", (host, port)).fetchall()

    def export(self, folder):
        folder = Path(folder)
        folder.mkdir(parents=True, exist_ok=True)
        # One snapshot across all exports preserves measurement/decision linkage.
        with self.connect() as db:
            db.execute("BEGIN")
            for table in ("samples", "decisions", "events"):
                rows = db.execute(f"SELECT * FROM {table} ORDER BY id").fetchall()
                fields = [d[0] for d in db.execute(f"SELECT * FROM {table} LIMIT 0").description]
                if table == "samples" and rows:
                    records = [dict(zip(fields, row)) for row in rows]
                    expanded = [dict(id=r["id"], cycle=r["cycle"], **json.loads(r["payload"])) for r in records]
                    with (folder / "samples.csv").open("w", newline="", encoding="utf-8-sig") as file:
                        fields = list(dict.fromkeys(key for row in expanded for key in row))
                        writer = csv.DictWriter(file, fieldnames=fields)
                        writer.writeheader()
                        writer.writerows(expanded)
                else:
                    with (folder / f"{table}.csv").open("w", newline="", encoding="utf-8-sig") as file:
                        writer = csv.writer(file)
                        writer.writerow(fields)
                        writer.writerows(rows)
        return folder
