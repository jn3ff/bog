use std::collections::HashSet;
use std::path::{Path, PathBuf};

use colored::Colorize;

use crate::ast::*;
use crate::parser;
use crate::treesitter::{self, Symbol};

#[derive(Debug)]
pub struct StubReport {
    pub stubs_generated: usize,
    pub files_modified: usize,
    pub files_created: usize,
}

/// Find .rs files that have functions not covered by .bog annotations.
/// Returns (source_path, bog_path, missing_symbols) tuples.
pub fn find_missing_annotations(root: &Path) -> Vec<(PathBuf, PathBuf, Vec<Symbol>)> {
    let mut results = Vec::new();

    let pattern = root.join("**/*.rs");
    let Ok(paths) = glob::glob(&pattern.to_string_lossy()) else {
        return results;
    };

    for source_path in paths.flatten() {
        // Skip build artifacts and git internals
        let rel = source_path.strip_prefix(root).unwrap_or(&source_path);
        if rel.components().any(|c| {
            matches!(c.as_os_str().to_str(), Some("target" | ".git"))
        }) {
            continue;
        }

        let bog_path = PathBuf::from(format!("{}.bog", source_path.display()));

        // Extract symbols from source
        let source = match std::fs::read_to_string(&source_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let symbols = match treesitter::extract_symbols(&source) {
            Ok(s) => s,
            Err(_) => continue,
        };

        if symbols.is_empty() {
            continue;
        }

        // Find which functions are already annotated
        let annotated: HashSet<String> = if bog_path.exists() {
            let content = match std::fs::read_to_string(&bog_path) {
                Ok(s) => s,
                Err(_) => String::new(),
            };
            match parser::parse_bog(&content) {
                Ok(bog) => bog
                    .annotations
                    .iter()
                    .filter_map(|a| {
                        if let Annotation::Fn(f) = a {
                            Some(f.name.clone())
                        } else {
                            None
                        }
                    })
                    .collect(),
                Err(_) => HashSet::new(),
            }
        } else {
            HashSet::new()
        };

        let missing: Vec<Symbol> = symbols
            .into_iter()
            .filter(|s| {
                // Skip already-annotated functions
                if annotated.contains(&s.name) {
                    return false;
                }
                // Skip impl methods (trait impls like fmt, eq, etc.)
                if s.kind == treesitter::SymbolKind::Method {
                    return false;
                }
                true
            })
            .collect();

        if !missing.is_empty() {
            results.push((source_path, bog_path, missing));
        }
    }

    results
}

/// Generate a stub annotation string for a symbol.
pub fn generate_stub(symbol: &Symbol) -> String {
    let deps_str = if symbol.calls.is_empty() {
        String::new()
    } else {
        let deps: Vec<String> = symbol.calls.iter().map(|c| c.to_string()).collect();
        format!(",\n  deps = [{}]", deps.join(", "))
    };

    format!(
        "#[fn({}) {{\n  status = yellow,\n  stub = true{},\n  description = \"TODO\"\n}}]",
        symbol.name, deps_str
    )
}

/// Generate a minimal file header for a new .bog sidecar.
fn generate_file_header(
    source_path: &Path,
    root: &Path,
) -> String {
    // Try to determine subsystem and owner from repo.bog
    let (owner, subsystem) = match find_subsystem_for_file(source_path, root) {
        Some((o, s)) => (o, s),
        None => ("unknown-agent".to_string(), "unknown".to_string()),
    };

    let today = chrono::Local::now().format("%Y-%m-%d");

    format!(
        r#"#[file(
  owner = "{owner}",
  subsystem = "{subsystem}",
  updated = "{today}",
  status = yellow
)]

#[description {{
  TODO
}}]

#[health(
  staleness = yellow
)]
"#
    )
}

/// Match a source file path against repo.bog subsystem declarations to find owner/subsystem.
fn find_subsystem_for_file(source_path: &Path, root: &Path) -> Option<(String, String)> {
    let repo_bog_path = root.join("repo.bog");
    let content = std::fs::read_to_string(repo_bog_path).ok()?;
    let bog = parser::parse_bog(&content).ok()?;

    let rel_path = source_path
        .strip_prefix(root)
        .unwrap_or(source_path)
        .to_string_lossy();

    for ann in &bog.annotations {
        if let Annotation::Subsystem(s) = ann {
            for pattern in &s.files {
                if let Ok(glob) = glob::Pattern::new(pattern) {
                    if glob.matches(&rel_path) {
                        return Some((s.owner.clone(), s.name.clone()));
                    }
                }
            }
        }
    }

    None
}

/// Generate stubs for all unannotated functions and write them to .bog files.
pub fn apply_stubs(root: &Path) -> StubReport {
    let missing = find_missing_annotations(root);
    let mut report = StubReport {
        stubs_generated: 0,
        files_modified: 0,
        files_created: 0,
    };

    for (source_path, bog_path, symbols) in &missing {
        let mut content = if bog_path.exists() {
            std::fs::read_to_string(bog_path).unwrap_or_default()
        } else {
            String::new()
        };

        let created = !bog_path.exists();

        // If no .bog file exists, generate a header
        if content.is_empty() {
            content = generate_file_header(source_path, root);
        }

        // Ensure trailing newline before appending stubs
        if !content.ends_with('\n') {
            content.push('\n');
        }

        // Append stubs
        for sym in symbols {
            content.push('\n');
            content.push_str(&generate_stub(sym));
            content.push('\n');
            report.stubs_generated += 1;
        }

        if let Err(e) = std::fs::write(bog_path, &content) {
            eprintln!(
                "  {} failed to write {}: {e}",
                "error:".red(),
                bog_path.display()
            );
            continue;
        }

        if created {
            report.files_created += 1;
        } else {
            report.files_modified += 1;
        }
    }

    report
}

/// Find all stub annotations across the project. Returns (file_path, fn_name) pairs.
pub fn list_stubs(root: &Path) -> Vec<(String, String)> {
    let mut stubs = Vec::new();

    let pattern = root.join("**/*.bog");
    let Ok(paths) = glob::glob(&pattern.to_string_lossy()) else {
        return stubs;
    };

    for bog_path in paths.flatten() {
        let rel = bog_path.strip_prefix(root).unwrap_or(&bog_path);
        if rel.components().any(|c| {
            matches!(c.as_os_str().to_str(), Some("target" | ".git"))
        }) {
            continue;
        }

        let content = match std::fs::read_to_string(&bog_path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let bog = match parser::parse_bog(&content) {
            Ok(b) => b,
            Err(_) => continue,
        };

        let file_str = rel.to_string_lossy().to_string();
        for ann in &bog.annotations {
            if let Annotation::Fn(f) = ann {
                if f.stub {
                    stubs.push((file_str.clone(), f.name.clone()));
                }
            }
        }
    }

    stubs
}
