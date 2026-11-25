//! Runs every case of the API v3 contract kit (`conformance/`) as its own test.
//!
//! `cargo test --test conformance` lists one test per kit case, named `<case id> <title>`,
//! plus two kit checks: the copy matches its MANIFEST, and every case of `conformance.md`
//! has a test here and a row in `CONFORMANCE.md`.

mod common;

#[path = "conformance/models.rs"]
mod models;
#[path = "conformance/rest.rs"]
mod rest;
#[path = "conformance/sessions.rs"]
mod sessions;
#[path = "conformance/webhooks.rs"]
mod webhooks;

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;

use libtest_mimic::{Arguments, Failed, Trial};
use sha2::{Digest, Sha256};

fn all_cases() -> Vec<common::Case> {
    let mut cases = Vec::new();
    cases.extend(models::cases());
    cases.extend(rest::cases());
    cases.extend(webhooks::cases());
    cases.extend(sessions::cases());
    cases
}

/// Case ids and titles from `conformance.md`.
fn kit_cases() -> BTreeMap<String, String> {
    let text = common::load_text("conformance.md");
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split('|').map(str::trim).collect();
        if cols.len() > 3 {
            let id = cols[1];
            let b = id.as_bytes();
            if b.len() == 4
                && b[0].is_ascii_uppercase()
                && b[1] == b'-'
                && b[2..].iter().all(u8::is_ascii_digit)
            {
                out.insert(id.to_owned(), cols[2].to_owned());
            }
        }
    }
    out
}

fn manifest_check() -> Result<(), Failed> {
    let dir = common::kit_dir();
    let manifest =
        std::fs::read_to_string(dir.join("MANIFEST")).map_err(|e| Failed::from(e.to_string()))?;
    let mut listed = BTreeSet::new();
    let mut problems = Vec::new();
    for line in manifest.lines().filter(|l| !l.trim().is_empty()) {
        let Some((digest, rel)) = line.split_once("  ") else {
            problems.push(format!("malformed MANIFEST line: {line}"));
            continue;
        };
        listed.insert(rel.to_owned());
        match std::fs::read(dir.join(rel)) {
            Ok(bytes) if hex::encode(Sha256::digest(&bytes)) == digest => {}
            Ok(_) => problems.push(format!("checksum differs: {rel}")),
            Err(_) => problems.push(format!("missing: {rel}")),
        }
    }
    let mut stack = vec![dir.clone()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(&dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if rel != "MANIFEST" && !listed.contains(&rel) {
                    problems.push(format!("not in MANIFEST: {rel}"));
                }
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(Failed::from(format!(
            "conformance/ does not match its MANIFEST (run scripts/sync-conformance.sh):\n  {}",
            problems.join("\n  ")
        )))
    }
}

fn coverage_check(registered: BTreeSet<String>) -> Result<(), Failed> {
    let kit: BTreeSet<String> = kit_cases().into_keys().collect();
    let missing: Vec<_> = kit.difference(&registered).cloned().collect();
    let extra: Vec<_> = registered.difference(&kit).cloned().collect();
    let table = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/CONFORMANCE.md"))
        .unwrap_or_default();
    let unmapped: Vec<_> = kit
        .iter()
        .filter(|id| !table.contains(&format!("| {id} |")))
        .cloned()
        .collect();
    let total = common::load_text("conformance.md");
    let declared = total.lines().find_map(|l| {
        l.strip_prefix("Total: ")
            .and_then(|r| r.split(' ').next()?.parse::<usize>().ok())
    });
    if !missing.is_empty()
        || !extra.is_empty()
        || !unmapped.is_empty()
        || declared != Some(kit.len())
    {
        return Err(Failed::from(format!(
            "kit cases without a test: {missing:?}; tests without a kit case: {extra:?}; \
             cases missing in CONFORMANCE.md: {unmapped:?}; declared total {declared:?}, found {}",
            kit.len()
        )));
    }
    Ok(())
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else {
        "test panicked".to_owned()
    }
}

fn main() {
    let args = Arguments::from_args();
    let titles = kit_cases();
    let mut registered = BTreeSet::new();
    let mut trials = Vec::new();
    for (id, case) in all_cases() {
        assert!(
            registered.insert(id.to_owned()),
            "case {id} registered twice"
        );
        let title = titles
            .get(id)
            .cloned()
            .unwrap_or_else(|| "(not in the kit)".to_owned());
        let case = Mutex::new(Some(case));
        trials.push(Trial::test(format!("{id} {title}"), move || {
            let case = case.lock().unwrap().take().expect("each case runs once");
            catch_unwind(AssertUnwindSafe(case)).map_err(|p| Failed::from(panic_message(p)))
        }));
    }
    trials.push(Trial::test(
        "kit: conformance/ matches its MANIFEST",
        manifest_check,
    ));
    trials.push(Trial::test(
        "kit: every case has a test and a CONFORMANCE.md row",
        move || coverage_check(registered.clone()),
    ));
    libtest_mimic::run(&args, trials).exit();
}
