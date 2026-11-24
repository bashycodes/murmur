//! On-screen name candidates on 10 synthetic chat screens
//! (`fixtures/screen-names/chat-screens.json`, all invented): the planted
//! names survive and the list stays short.

use std::sync::Arc;

use murmur_core::asr::{build_initial_prompt, MockAsr};
use murmur_core::config::{FieldContext, SessionConfig};
use murmur_core::dictionary::Dictionary;
use murmur_core::screen_names::{name_candidates, MAX_CANDIDATES, MAX_CANDIDATE_WORDS};
use murmur_core::MurmurCore;
use serde_json::Value;

fn screens() -> Vec<Value> {
    let raw = include_str!("../fixtures/screen-names/chat-screens.json");
    let v: Value = serde_json::from_str(raw).unwrap();
    v["screens"].as_array().unwrap().clone()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

/// Letter words, lower case ("Harrow & Vane" → ["harrow", "and", "vane"]).
fn words(s: &str) -> Vec<String> {
    s.replace('&', " and ")
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'')
        .filter(|w| w.chars().any(char::is_alphabetic))
        .map(str::to_lowercase)
        .collect()
}

/// `name` is in a candidate as consecutive words.
fn survives(name: &str, candidates: &[String]) -> bool {
    let n = words(name);
    candidates
        .iter()
        .any(|c| words(c).windows(n.len()).any(|w| w == n.as_slice()))
}

#[test]
fn planted_names_survive_and_the_list_stays_short() {
    let all = screens();
    assert_eq!(all.len(), 10);
    let (mut planted, mut kept, mut sent) = (0, 0, 0);
    for s in &all {
        let got = name_candidates(&strings(&s["screen"]));
        assert!(got.len() <= MAX_CANDIDATES, "{}: {got:?}", s["id"]);
        assert!(
            got.iter()
                .all(|c| c.split_whitespace().count() <= MAX_CANDIDATE_WORDS),
            "{}: {got:?}",
            s["id"]
        );
        for n in strings(&s["names"]) {
            planted += 1;
            if survives(&n, &got) {
                kept += 1;
            } else {
                eprintln!("{}: lost {n:?} from {got:?}", s["id"]);
            }
        }
        sent = sent.max(got.len());
    }
    // ≥ 90 % of the planted names, ≤ 60 entries on any screen.
    assert!(kept * 10 >= planted * 9, "{kept}/{planted} names kept");
    assert!(sent <= 60, "{sent} entries");
}

#[test]
fn a_whole_screen_shrinks_to_names() {
    let s = &screens()[1];
    let raw = strings(&s["screen"]);
    let got = name_candidates(&raw);
    let raw_chars: usize = raw.iter().map(|e| e.chars().count()).sum();
    let got_chars: usize = got.iter().map(|e| e.chars().count()).sum();
    assert!(
        got_chars * 4 < raw_chars,
        "{got_chars} of {raw_chars} chars: {got:?}"
    );
    assert!(!got.iter().any(|c| c.contains("cabin")));
}

#[test]
fn a_name_in_the_newest_bubble_of_a_full_screen_reaches_the_prompt() {
    // A full capture (the phone's 2000-char cap): older bubbles above,
    // the planted name in the last (newest) one.
    let mut screen: Vec<String> = Vec::new();
    let mut chars = 0;
    'fill: for s in screens() {
        for e in strings(&s["screen"]) {
            if chars + e.chars().count() > 1940 {
                break 'fill;
            }
            chars += e.chars().count();
            screen.push(e);
        }
    }
    screen.push("ok see you there, ask Ottoline Reyes too".into());
    assert!(screen.iter().map(|e| e.chars().count()).sum::<usize>() > 1900);
    let none = Dictionary::default();
    // Whole bubbles: the budget is spent on the oldest ones.
    assert!(!build_initial_prompt(&none, &screen).contains("Ottoline Reyes"));
    let names = name_candidates(&screen);
    assert_eq!(names[0], "Ottoline Reyes");
    assert!(build_initial_prompt(&none, &names).contains("Ottoline Reyes"));
}

#[test]
fn the_core_biases_asr_with_candidates_not_bubbles() {
    // MurmurCore::set_context is where screen words enter (Android,
    // Windows and murmur-server sessions).
    let raw = strings(&screens()[0]["screen"]);
    let asr = Arc::new(MockAsr::new("hello there"));
    let core = MurmurCore::from_engines(asr.clone(), None);
    core.start_session(SessionConfig::default()).unwrap();
    core.set_context(FieldContext {
        screen_words: raw.clone(),
        ..Default::default()
    });
    for c in speech(400).chunks(640) {
        core.push_audio(c);
    }
    core.commit();
    let req = asr.last_request().expect("asr ran");
    let want = name_candidates(&raw);
    assert_eq!(req.screen_terms, want);
    assert_eq!(
        req.initial_prompt,
        build_initial_prompt(&Dictionary::default(), &want)
    );
    assert!(!req.initial_prompt.contains("climbing gym"));
}

fn speech(ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}
