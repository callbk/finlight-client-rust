//! R-DOCS-1: the README follows the shared skeleton; the translations keep its structure and
//! code blocks. The README code itself is compiled as doctests (`src/lib.rs`).

const SKELETON: [&str; 17] = [
    "# finlight client for Rust",
    "## Versions",
    "## Requirements",
    "## Installation",
    "## Quick start",
    "## Search articles",
    "## Look up articles, companies and sources",
    "## Absent, null and empty",
    "## Real-time streams",
    "## Webhooks",
    "## Errors and retries",
    "## Configuration",
    "## Logging",
    "## Data notes",
    "## Migrating from 0.1",
    "## Development",
    "## License",
];

fn read(name: &str) -> String {
    std::fs::read_to_string(format!("{}/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// Headings outside code blocks.
fn headings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_code = false;
    for line in text.lines() {
        if line.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if !in_code && line.starts_with('#') {
            out.push(line.to_owned());
        }
    }
    out
}

fn code_blocks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if line.starts_with("```") {
            match current.take() {
                Some(block) => out.push(block),
                None => current = Some(String::from(line)),
            }
            continue;
        }
        if let Some(block) = current.as_mut() {
            block.push('\n');
            block.push_str(line);
        }
    }
    out
}

fn level(h: &str) -> usize {
    h.chars().take_while(|c| *c == '#').count()
}

#[test]
fn readme_follows_the_skeleton() {
    let readme = read("README.md");
    assert_eq!(headings(&readme), SKELETON);
    let versions = readme.find("## Versions").unwrap();
    let breaking = readme
        .find("> **Breaking release.**")
        .expect("breaking notice");
    assert!(
        breaking < versions,
        "the breaking notice comes before Versions"
    );
    assert!(readme.contains("| SDK version | API version | Status |"));
    assert!(readme.contains("| 0.2.x | v3"));
    assert!(readme.contains("| 0.1.x | v2"));
    assert!(readme.contains("Rust 1.85"));
    assert!(readme.contains("cargo add finlight-client@0.2"));
    for must in [
        "32,000 bytes",
        "2 h",
        "Initial push",
        "Connection limit",
        "410",
        "event_id",
        "WrongPayloadVersion",
    ] {
        assert!(readme.contains(must), "README mentions {must}");
    }
}

#[test]
fn translations_keep_structure_and_code() {
    let readme = read("README.md");
    let en_headings = headings(&readme);
    let en_code = code_blocks(&readme);
    for name in ["README.zh-CN.md", "README.ja.md", "README.ko.md"] {
        let text = read(name);
        let h = headings(&text);
        assert_eq!(
            h.iter().map(|x| level(x)).collect::<Vec<_>>(),
            en_headings.iter().map(|x| level(x)).collect::<Vec<_>>(),
            "{name}: heading levels"
        );
        assert_eq!(code_blocks(&text), en_code, "{name}: code blocks");
        assert!(
            text.contains("> **Breaking release.**"),
            "{name}: breaking notice"
        );
        assert!(
            text.contains("| SDK version | API version | Status |"),
            "{name}: version table"
        );
        let notice_line = text
            .lines()
            .take(10)
            .any(|l| l.starts_with('*') && l.contains("README") && !l.contains('|'));
        assert!(notice_line, "{name}: translation notice at the top");
    }
}

#[test]
fn changelog_starts_with_the_breaking_note() {
    let changelog = read("CHANGELOG.md");
    let first_entry = changelog.split("\n## ").nth(1).unwrap();
    assert!(first_entry.starts_with("0.2.0"));
    assert!(first_entry.contains("Breaking"));
    let manifest = read("Cargo.toml");
    assert!(manifest.contains("description = \"Breaking release"));
}
