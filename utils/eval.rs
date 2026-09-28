//! Scores the cascade against the hand-labelled corpus.
//!
//! `just eval`       — replay from evals/recordings, compare to the baseline
//! `just eval-live`  — call jev, rewrite the recordings, then score
//! `just eval-bless` — replay and overwrite the baseline with the result
//!
//! The scoring logic (`score`, `tally`, the baseline-drift diff) lives in
//! `cmt::eval`, where `cargo test` can reach it; this binary is the CLI and
//! the live/bless side effects.

use std::collections::BTreeMap;
use std::path::Path;

use cmt::eval::{self, Outcome};
use cmt::evalcase::Case;
use cmt::intent::{self, Answers, Band};
use cmt::jev;
use cmt::message::CommitType;

fn main() {
    let live = std::env::args().any(|a| a == "--live");
    let bless = std::env::args().any(|a| a == "--bless");

    let cases = match Case::load_dir(Path::new(eval::CASES)) {
        Ok(cases) => cases,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    if cases.is_empty() {
        eprintln!(
            "no cases in {}; run `just harvest` and label the output",
            eval::CASES
        );
        std::process::exit(1);
    }

    if live {
        let transport = match jev::Http::from_env() {
            Ok(transport) => transport,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        };
        if let Err(e) = std::fs::create_dir_all(eval::RECORDINGS) {
            eprintln!("{}: {e}", eval::RECORDINGS);
            std::process::exit(1);
        }
        for case in &cases {
            let state = jev::State {
                files: case.files.clone(),
                diff: case.diff.clone(),
            };
            let request = match serde_json::to_string(&jev::Request {
                state: &state,
                model: jev::MODEL,
                questions: intent::questions(case.has_tests()),
            }) {
                Ok(request) => request,
                Err(e) => {
                    eprintln!("{}: serialize: {e}", case.name);
                    continue;
                }
            };
            match jev::Transport::post(&transport, &request) {
                Ok(raw) => match std::fs::write(eval::recording_path(&case.name), &raw) {
                    Ok(()) => println!("recorded {}", case.name),
                    Err(e) => eprintln!("{}: write recording: {e}", case.name),
                },
                Err(e) => eprintln!("{}: {e}", case.name),
            }
        }
    }

    let mut token_counts: Vec<u64> = Vec::new();
    let answers: BTreeMap<String, Answers> = cases
        .iter()
        .filter_map(|c| {
            let (answers, tokens) = eval::recording(&c.name)?;
            token_counts.push(tokens);
            Some((c.name.clone(), answers))
        })
        .collect();

    let mut correct = 0usize;
    let mut wrong = 0usize;
    let mut refused = 0usize;
    let mut unrecorded = 0usize;
    let mut baseline_correct = 0usize;
    let mut blame: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut current: BTreeMap<String, String> = BTreeMap::new();

    for case in &cases {
        if case.expected == CommitType::Refactor {
            baseline_correct += 1;
        }
        let outcome = eval::score(case, answers.get(&case.name), intent::DEFAULT_BAND);
        current.insert(case.name.clone(), outcome.label());
        match &outcome {
            Outcome::Decided { kind, .. } if *kind == case.expected => correct += 1,
            Outcome::Decided { read, .. } => {
                wrong += 1;
                if let Some(last) = read
                    .iter()
                    .rev()
                    .find(|id| **id != intent::CONSUMER_MUST_CHANGE)
                {
                    *blame.entry(last).or_default() += 1;
                }
            }
            Outcome::Refused { .. } => refused += 1,
            Outcome::Unrecorded => unrecorded += 1,
        }
    }

    let total = cases.len();
    let answered = correct + wrong;
    println!("\n{total} cases\n");
    println!(
        "{:<14}{:>9}{:>8}{:>10}{:>11}{:>11}",
        "", "correct", "wrong", "refused", "accuracy", "coverage"
    );
    println!(
        "{:<14}{:>9}{:>8}{:>10}{:>11.3}{:>11.3}",
        "jev",
        correct,
        wrong,
        refused,
        if answered == 0 {
            0.0
        } else {
            correct as f64 / answered as f64
        },
        answered as f64 / total as f64,
    );
    println!(
        "{:<14}{:>9}{:>8}{:>10}{:>11.3}{:>11.3}",
        "mechanical",
        baseline_correct,
        total - baseline_correct,
        0,
        baseline_correct as f64 / total as f64,
        1.0,
    );
    println!(
        "\nmechanical is the always-`refactor` baseline: the floor a network call must clear."
    );
    if unrecorded > 0 {
        println!("\n{unrecorded} of {total} cases have no recording; run `just eval-live`");
    }
    if !blame.is_empty() {
        println!("\nwrong by predicate");
        for (predicate, count) in &blame {
            println!("  {predicate:<32}{count}");
        }
    }
    if !token_counts.is_empty() {
        let sum: u64 = token_counts.iter().sum();
        let mean = sum as f64 / token_counts.len() as f64;
        println!(
            "\ntokens: {sum} input tokens across {} recordings, {mean:.0} mean",
            token_counts.len()
        );
    }

    println!("\ndead band sweep");
    println!("  {:<14}{:>10}{:>11}", "band", "coverage", "accuracy");
    for (low, high) in [
        (0.10, 0.90),
        (0.20, 0.80),
        (0.25, 0.75),
        (0.35, 0.65),
        (0.40, 0.60),
    ] {
        let (c, a, t) = eval::tally(&cases, &answers, Band { low, high });
        println!(
            "  {:<14}{:>10.3}{:>11.3}",
            format!("{low:.2}-{high:.2}"),
            a as f64 / t as f64,
            if a == 0 { 0.0 } else { c as f64 / a as f64 },
        );
    }

    if bless {
        let rendered: String = current
            .iter()
            .map(|(name, label)| format!("{name} = \"{label}\"\n"))
            .collect();
        if let Err(e) = std::fs::write(eval::BASELINE, rendered) {
            eprintln!("{}: {e}", eval::BASELINE);
            std::process::exit(1);
        }
        println!("\nbaseline rewritten with {} entries", current.len());
        return;
    }

    if answers.is_empty() && !Path::new(eval::BASELINE).exists() {
        println!(
            "\n0 of {total} cases recorded; the jev row is unmeasured. Run `just eval-live` \
             with TYPESAFE_API_KEY set, then `just eval-bless`."
        );
        return;
    }

    let expected = eval::read_baseline();
    if expected.is_empty() {
        eprintln!(
            "\nno baseline at {}; run `just eval-bless` once the report looks right",
            eval::BASELINE
        );
        std::process::exit(1);
    }
    let drift = eval::drift(&current, &expected);
    if drift.is_empty() {
        println!("\nbaseline matches");
        return;
    }
    eprintln!("\nbaseline drift:");
    for (name, label) in &drift {
        eprintln!(
            "  {name}: baseline {:?}, now {label:?}",
            expected.get(name).map(String::as_str).unwrap_or("(absent)")
        );
    }
    eprintln!("fix the cascade, or run `just eval-bless` if the new result is correct");
    std::process::exit(1);
}
