//! Replays every fixture recorded from the TypeScript domain by
//! `scripts/gen-parity-fixtures.test.ts` and requires identical JSON output,
//! or the identical error message.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use ledger_core::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureEnv {
    next_id: u64,
    now: String,
    today: String,
}

#[derive(Deserialize)]
struct Fixture {
    name: String,
    call: String,
    env: FixtureEnv,
    input: Map<String, Value>,
    /// Present (possibly `null`) when the call returned.
    #[serde(default, deserialize_with = "present")]
    output: Option<Value>,
    error: Option<String>,
}

fn present<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/parity")
}

fn read_json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Expands `{"$state": name}` and `{"$concat": [...]}` recipes.
fn resolve(value: Value, states: &mut BTreeMap<String, Value>) -> Value {
    match value {
        Value::Object(map) if map.contains_key("$state") => {
            let name = map["$state"].as_str().unwrap().to_string();
            states
                .entry(name.clone())
                .or_insert_with(|| read_json(&fixtures_dir().join(format!("states/{name}.json"))))
                .clone()
        }
        Value::Object(map) if map.contains_key("$concat") => {
            let mut text = String::new();
            for part in map["$concat"].as_array().unwrap() {
                match part {
                    Value::String(s) => text.push_str(s),
                    Value::Object(p) => {
                        let spec = p["$repeat"].as_array().unwrap();
                        let count = spec[1].as_u64().unwrap() as usize;
                        text.push_str(&spec[0].as_str().unwrap().repeat(count));
                    }
                    other => panic!("bad $concat part {other}"),
                }
            }
            Value::String(text)
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, resolve(v, states)))
                .collect(),
        ),
        Value::Array(items) => {
            Value::Array(items.into_iter().map(|v| resolve(v, states)).collect())
        }
        other => other,
    }
}

struct Args(Map<String, Value>);

impl Args {
    fn get<T: DeserializeOwned>(&self, key: &str) -> T {
        let value = self.0.get(key).cloned().unwrap_or(Value::Null);
        serde_json::from_value(value).unwrap_or_else(|e| panic!("argument {key}: {e}"))
    }
    fn str(&self, key: &str) -> String {
        self.get(key)
    }

    fn business(&self) -> BusinessData {
        if !self.0.contains_key("journalContexts") {
            return self.get("business");
        }
        // A few negative query fixtures deliberately mix already-validated
        // journals or remove categories in memory. Rebuild those journals in
        // their recorded original contexts; never weaken production loading
        // to accept these deliberately inconsistent business snapshots.
        let contexts: Vec<BusinessData> = self
            .get::<Vec<String>>("journalContexts")
            .into_iter()
            .map(|name| {
                serde_json::from_value(read_json(
                    &fixtures_dir().join(format!("states/{name}.json")),
                ))
                .unwrap()
            })
            .collect();
        let mut raw: Value = self.get("business");
        let entries: Vec<UncheckedJournal> =
            serde_json::from_value(raw["journals"].take()).unwrap();
        raw["journals"] = Value::Array(Vec::new());
        let mut business: BusinessData = serde_json::from_value(raw).unwrap();
        business.journals = entries
            .into_iter()
            .map(|entry| {
                let context = contexts
                    .iter()
                    .find(|b| b.profile.id == entry.business_id)
                    .expect("fixture must supply the original journal context");
                JournalEntry::new(context, entry)
                    .expect("fixture journals must be valid in their original context")
            })
            .collect();
        business
    }
}

fn ok<T: Serialize>(value: T) -> std::result::Result<Value, String> {
    Ok(serde_json::to_value(value).unwrap())
}

fn out<T: Serialize>(result: Result<T>) -> std::result::Result<Value, String> {
    result
        .map(|v| serde_json::to_value(v).unwrap())
        .map_err(|e| e.to_string())
}

fn run(call: &str, a: &Args, env: &mut SequentialEnv) -> std::result::Result<Value, String> {
    let business = || a.business();
    match call {
        "parseMoney" => out(parse_money(&a.str("input"))),
        "amountInput" => ok(amount_input(a.get("n"))),
        "validDate" => ok(valid_date(&a.str("date"))),
        "normalizeImportDate" => ok(normalize_import_date(&a.str("value"), a.get("format"))),
        "validation" => ok(validation(&business(), &a.get("draft"))),
        "missingDetails" => ok(missing_details(&business(), &a.get("draft"))),
        "duplicates" => ok(duplicates(&business(), &a.get("draft"))),
        "assertBalanced" => out(assert_balanced(&business(), &a.get("entry"))),
        "blankDraft" => ok(blank_draft(&business(), env)),
        "currentPeriod" => ok(current_period(env)),
        "saveDraft" => out(save_draft(&business(), &a.get("draft"), env)),
        "event" => ok(event(
            &business(),
            env,
            a.get("type"),
            &a.str("entityId"),
            &a.str("detail"),
            a.get("sourceIds"),
        )),
        "approve" => out(approve(&business(), &a.str("expenseId"), env)),
        "correct" => out(correct(
            &business(),
            &a.str("originalId"),
            &a.get("replacement"),
            &a.str("reason"),
            env,
        )),
        "totals" => out(totals(&business(), &a.str("period"))),
        "categoryTotals" => out(category_totals(
            &business(),
            &a.str("period"),
            a.get("currency"),
        )),
        "parseCSV" => out(parse_csv(&a.str("text"))),
        "guessMapping" => ok(guess_mapping(&a.get::<Vec<String>>("headers"))),
        "previewImport" => out(preview_import(
            &business(),
            &a.get::<Vec<Vec<String>>>("rows"),
            &a.get("map"),
            &a.str("accountId"),
            a.get("convention"),
            &a.get::<Option<ImportOptions>>("options")
                .unwrap_or_default(),
            env,
        )),
        "commitImport" => out(commit_import(
            &business(),
            &a.get::<Vec<ImportRow>>("rows"),
            &a.str("accountId"),
            env,
        )),
        "updateSettings" => out(update_settings(&business(), &a.get("change"), env)),
        "saveRule" => out(save_rule(&business(), &a.get("input"), env)),
        "suggest" => out(suggest(&business(), &a.get("input"), env)),
        "recordTransfer" => out(record_transfer(&business(), &a.get("input"), env)),
        "correctTransfer" => out(correct_transfer(
            &business(),
            &a.str("originalId"),
            &a.get("replacement"),
            &a.str("reason"),
            env,
        )),
        "recordOpeningBalance" => out(record_opening_balance(&business(), &a.get("input"), env)),
        "balances" => out(balances(&business(), &a.str("asOf"), a.get("currency"))),
        "encodeCSV" => ok(encode_csv(&a.get::<Vec<Vec<String>>>("rows"))),
        "exportBalances" => out(export_balances(
            &business(),
            &a.str("asOf"),
            a.get("currency"),
        )),
        "exportRecords" => out(export_records(
            &business(),
            a.get("format"),
            &a.get("filter"),
        )),
        other => panic!("unknown call {other}"),
    }
}

/// The first JSON path where `got` and `want` differ.
fn first_difference(path: String, got: &Value, want: &Value) -> Option<String> {
    match (got, want) {
        (Value::Object(g), Value::Object(w)) => {
            for key in g.keys().chain(w.keys()) {
                let (gv, wv) = (g.get(key), w.get(key));
                if gv != wv {
                    return match (gv, wv) {
                        (Some(gv), Some(wv)) => first_difference(format!("{path}.{key}"), gv, wv),
                        _ => Some(format!("{path}.{key}: got {gv:?}, want {wv:?}")),
                    };
                }
            }
            None
        }
        (Value::Array(g), Value::Array(w)) if g.len() == w.len() => g
            .iter()
            .zip(w)
            .enumerate()
            .find_map(|(i, (gv, wv))| first_difference(format!("{path}[{i}]"), gv, wv)),
        _ if got == want => None,
        _ => {
            let clip = |v: &Value| v.to_string().chars().take(300).collect::<String>();
            Some(format!("{path}: got {}, want {}", clip(got), clip(want)))
        }
    }
}

#[test]
fn rust_matches_every_typescript_fixture() {
    let mut states = BTreeMap::new();
    let mut areas: Vec<PathBuf> = fs::read_dir(fixtures_dir())
        .expect("fixtures/parity exists; run `npm run parity:fixtures`")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    areas.sort();
    let (mut total, mut failures) = (0, Vec::new());
    for path in &areas {
        let area = path.file_stem().unwrap().to_string_lossy().to_string();
        let fixtures: Vec<Fixture> = serde_json::from_value(read_json(path)).unwrap();
        assert!(!fixtures.is_empty(), "{area} has no fixtures");
        for f in fixtures {
            total += 1;
            let input = match resolve(Value::Object(f.input), &mut states) {
                Value::Object(map) => map,
                _ => unreachable!(),
            };
            let mut env = SequentialEnv::new(f.env.next_id, f.env.now, f.env.today);
            let got = run(&f.call, &Args(input), &mut env);
            let problem = match (&got, &f.output, &f.error) {
                (Err(got), _, Some(want)) if got == want => None,
                (Ok(got), Some(want), None) => first_difference(String::from("output"), got, want),
                (got, _, Some(want)) => Some(format!("want error {want:?}, got {got:?}")),
                (Err(got), _, None) => Some(format!("unexpected error {got:?}")),
                (Ok(_), None, None) => Some("fixture has neither output nor error".into()),
            };
            if let Some(problem) = problem {
                failures.push(format!("{area} / {}: {problem}", f.name));
            }
        }
    }
    assert!(total >= 400, "only {total} fixtures found");
    assert!(
        failures.is_empty(),
        "{} of {total} fixtures differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_recorded_state_round_trips_to_identical_json() {
    let dir = fixtures_dir().join("states");
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let value = read_json(&path);
        let (value, again) = if path.file_stem().unwrap() == "seed" {
            let workspace: Workspace = serde_json::from_value(value["workspace"].clone()).unwrap();
            (
                value["workspace"].clone(),
                serde_json::to_value(workspace).unwrap(),
            )
        } else {
            let business: BusinessData = serde_json::from_value(value.clone()).unwrap();
            (value, serde_json::to_value(business).unwrap())
        };
        assert_eq!(
            first_difference(path.display().to_string(), &again, &value),
            None
        );
    }
}
