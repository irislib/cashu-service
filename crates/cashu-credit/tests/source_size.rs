use std::{fs, path::Path};

const MAX_RUST_LINES: usize = 1_000;

#[test]
fn workspace_rust_files_stay_below_size_limit() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest_dir
        .ancestors()
        .find(|path| {
            fs::read_to_string(path.join("Cargo.toml"))
                .is_ok_and(|manifest| manifest.contains("[workspace]"))
        })
        .unwrap_or(manifest_dir);
    let mut violations = Vec::new();
    inspect(workspace, workspace, &mut violations);
    assert!(
        violations.is_empty(),
        "Rust files must be at most {MAX_RUST_LINES} lines:\n{}",
        violations.join("\n")
    );
}

fn inspect(root: &Path, path: &Path, violations: &mut Vec<String>) {
    let entries = fs::read_dir(path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", path.display());
    });
    for entry in entries {
        let entry = entry.expect("failed to read workspace entry");
        let path = entry.path();
        if path.is_dir() {
            if !matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some(".git" | "target")
            ) {
                inspect(root, &path, violations);
            }
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("failed to read {}: {error}", path.display());
        });
        let lines = source.lines().count();
        if lines > MAX_RUST_LINES {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            violations.push(format!("{}: {lines}", relative.display()));
        }
    }
}
