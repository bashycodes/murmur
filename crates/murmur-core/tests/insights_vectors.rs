//! The golden Insights vectors shared with the Android shell
//! (`fixtures/insights/insights_vectors.json`, written by
//! `tools/gen-insights-vectors.py` and also read by the Kotlin
//! `InsightsVectorsTest`) must stay true for the Rust math, so the Kotlin
//! mirror can't drift from it unnoticed.

use murmur_core::insights::{streaks, Aggregates, LocalDate};
use serde_json::Value;

fn fixture() -> Value {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/insights/insights_vectors.json"
    );
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn day(v: &Value) -> LocalDate {
    LocalDate::parse(v.as_str().unwrap()).unwrap()
}

/// Structural equality with numbers compared within 1e-9.
fn approx_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            (x.as_f64().unwrap() - y.as_f64().unwrap()).abs() < 1e-9
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| approx_eq(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| approx_eq(v, w)))
        }
        _ => a == b,
    }
}

#[test]
fn summary_cases_match() {
    let f = fixture();
    let cases = f["cases"].as_array().unwrap();
    assert!(cases.len() >= 10);
    for c in cases {
        let mut g = Aggregates::default();
        for a in c["adds"].as_array().unwrap() {
            g.add(
                day(&a["date"]),
                a["words"].as_u64().unwrap(),
                a["ms"].as_u64().unwrap(),
                a["app"].as_str().unwrap(),
            );
        }
        if c["reset"].as_bool() == Some(true) {
            g.reset();
        }
        let got =
            serde_json::to_value(g.summary(day(&c["today"]), c["typing_wpm"].as_f64().unwrap()))
                .unwrap();
        assert!(
            approx_eq(&got, &c["expect"]),
            "case {}:\n got {got}\nwant {}",
            c["name"],
            c["expect"]
        );
    }
}

#[test]
fn streak_vectors_match() {
    for v in fixture()["streaks"].as_array().unwrap() {
        let days: Vec<LocalDate> = v["days"].as_array().unwrap().iter().map(day).collect();
        let want = (
            v["current"].as_u64().unwrap() as u32,
            v["longest"].as_u64().unwrap() as u32,
        );
        assert_eq!(streaks(&days, day(&v["today"])), want, "{v}");
    }
}

#[test]
fn local_date_vectors_match() {
    for v in fixture()["dates"].as_array().unwrap() {
        let got = LocalDate::from_unix_ms(
            v["ts_ms"].as_i64().unwrap(),
            v["offset_min"].as_i64().unwrap() as i32,
        );
        assert_eq!(got.key(), v["date"].as_str().unwrap(), "{v}");
    }
}
