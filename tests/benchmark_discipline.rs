use serde_json::Value;
use std::fs;
use std::path::Path;

#[test]
fn ranking_code_does_not_contain_benchmark_repo_names() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let benchmark: Value = serde_json::from_str(
        &fs::read_to_string(root.join("benchmarks/results/sifs-full.json")).unwrap(),
    )
    .unwrap();
    let ranking_sources = [
        fs::read_to_string(root.join("src/ranking.rs")).unwrap(),
        fs::read_to_string(root.join("src/search.rs")).unwrap(),
    ]
    .join("\n")
    .to_lowercase();
    let allow = [
        "click",
        "curl",
        "express",
        "flask",
        "gin",
        "plug",
        "rack",
        "rails",
        "redis",
        "serde",
        "tokio",
        "model2vec",
    ];

    for repo in benchmark["results"].as_array().unwrap() {
        let name = repo["repo"].as_str().unwrap().to_lowercase();
        if name.len() < 5 || allow.contains(&name.as_str()) {
            continue;
        }
        assert!(
            !ranking_sources.contains(&name),
            "production ranking/search code contains benchmark repo name {name:?}"
        );
    }
}
