//! Frozen fixtures evaluated by the existing Python implementation. Never loaded into a service database.
use nettime_next_native::{
    config::{Config, Server},
    protocol::Sample,
    selection::{Peer, assess},
};
use serde_json::Value;
#[test]
fn mitigation_matches_existing_implementation() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("mitigation-vectors.json")).unwrap();
    for case in cases {
        let cfg: Config = serde_json::from_value(case["cfg"].clone()).unwrap();
        let peers: Vec<_> = case["peers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                let s: Server = serde_json::from_value(v["server"].clone()).unwrap();
                let mut p = Peer::new(s, cfg.poll_seconds);
                for raw in v["samples"].as_array().unwrap() {
                    p.accept(serde_json::from_value::<Sample>(raw.clone()).unwrap());
                }
                p
            })
            .collect();
        let actual = assess(&peers, &cfg, case["now"].as_f64().unwrap());
        let expected = &case["expected"];
        assert_eq!(
            actual.ready,
            expected["ready"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        assert_eq!(
            actual.selected.as_deref(),
            expected["selected"].as_str(),
            "{}",
            case["name"]
        );
        assert_eq!(
            actual.survivors.len(),
            expected["survivors"].as_array().unwrap().len()
        );
        if let Some(offset) = expected["offset_ms"].as_f64() {
            assert!((actual.offset_ms.unwrap() - offset).abs() < 1e-9);
        }
        for c in &actual.candidates {
            let e = expected["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["peer"].as_str() == Some(&c.peer))
                .unwrap();
            assert!((c.distance_ms - e["distance_ms"].as_f64().unwrap()).abs() < 1e-9);
            assert!((c.jitter_ms - e["jitter_ms"].as_f64().unwrap()).abs() < 1e-9);
        }
    }
}
