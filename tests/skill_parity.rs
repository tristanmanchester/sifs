use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn frontmatter(path: &Path) -> BTreeMap<String, String> {
    let text = fs::read_to_string(path).expect("read skill");
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("---"),
        "{path:?} must start with frontmatter"
    );

    let mut values = BTreeMap::new();
    for line in lines {
        if line == "---" {
            return values;
        }
        if line.starts_with(' ') || line.trim().is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            values.insert(key.to_string(), value.trim().trim_matches('"').to_string());
        }
    }
    panic!("{path:?} missing closing frontmatter marker");
}

#[test]
fn skill_packages_have_valid_frontmatter() {
    let root = repo_root();
    let skill_paths = [
        "skills/sifs-search/SKILL.md",
        "extras/openclaw/sifs-search/SKILL.md",
        "extras/hermes/sifs-search/SKILL.md",
        "extras/agent-skills/sifs-search/SKILL.md",
    ];

    for rel in skill_paths {
        let path = root.join(rel);
        let values = frontmatter(&path);
        assert_eq!(values.get("name").map(String::as_str), Some("sifs-search"));
        let description = values.get("description").expect("description");
        assert!(description.len() <= 1024, "{rel} description is too long");
        assert!(
            description.starts_with("Use this skill when"),
            "{rel} should use trigger-oriented description wording"
        );
        assert!(
            description.contains("Do not use"),
            "{rel} description should include negative trigger guidance"
        );
        assert_eq!(values.get("license").map(String::as_str), Some("MIT"));
        assert!(
            values.contains_key("compatibility"),
            "{rel} missing compatibility"
        );
    }
}

#[test]
fn openclaw_package_is_self_contained_and_publishable() {
    let root = repo_root();
    let package = root.join("extras/openclaw/sifs-search");
    for rel in [
        "SKILL.md",
        "references/commands.md",
        "references/mcp.md",
        "references/troubleshooting.md",
        "scripts/check-setup.sh",
    ] {
        assert!(package.join(rel).exists(), "missing {rel}");
    }

    let values = frontmatter(&package.join("SKILL.md"));
    let metadata_raw = values.get("metadata").expect("OpenClaw metadata");
    let metadata: Value = serde_json::from_str(metadata_raw).expect("valid metadata JSON");
    let openclaw = metadata
        .get("openclaw")
        .and_then(Value::as_object)
        .expect("openclaw metadata object");
    assert_eq!(
        openclaw.get("version").and_then(Value::as_str),
        Some("0.1.0")
    );
    assert!(
        openclaw
            .get("requires")
            .and_then(|requires| requires.get("bins"))
            .and_then(Value::as_array)
            .is_some_and(|bins| bins.iter().any(|bin| bin.as_str() == Some("sifs"))),
        "OpenClaw metadata should declare the sifs binary requirement"
    );
}

#[test]
fn skill_evals_cover_positive_and_negative_triggers() {
    let evals_path = repo_root().join("skills/sifs-search/evals/evals.json");
    let raw = fs::read_to_string(evals_path).expect("read evals");
    let cases: Value = serde_json::from_str(&raw).expect("valid eval JSON");
    let cases = cases.as_array().expect("eval file is an array");
    assert!(
        cases.len() >= 6,
        "expected positive and negative trigger cases"
    );

    let positives = cases
        .iter()
        .filter(|case| {
            case.get("expected_behavior")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains("Uses the sifs-search skill"))
        })
        .count();
    let negatives = cases
        .iter()
        .filter(|case| {
            case.get("expected_behavior")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains("Does not use the sifs-search skill"))
        })
        .count();

    assert!(
        positives >= 1,
        "expected at least one positive trigger eval"
    );
    assert!(negatives >= 2, "expected multiple negative trigger evals");
}
