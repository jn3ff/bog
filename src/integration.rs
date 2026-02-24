use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::process::Command;

use colored::Colorize;
use serde::Deserialize;

use crate::ast::*;
use crate::stub;
use crate::treesitter;

#[derive(Debug, thiserror::Error)]
pub enum IntegrationError {
    #[error("Command failed: {0}")]
    CommandFailed(String),

    #[error("Failed to write {0}: {1}")]
    WriteFailed(String, String),
}

#[derive(Debug, Clone)]
pub struct IntegrationFinding {
    pub file_path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub code: String,
    pub level: FindingLevel,
    pub message: String,
    pub rendered: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FindingLevel {
    Warning,
}

#[derive(Debug)]
pub struct IntegrationReport {
    pub skimsystem: String,
    pub integration_name: String,
    pub total_findings: usize,
    pub findings_by_subsystem: HashMap<String, Vec<IntegrationFinding>>,
    pub unowned_findings: Vec<IntegrationFinding>,
    pub files_written: usize,
    pub change_requests_generated: usize,
    pub build_error: Option<String>,
}

// --- Cargo diagnostic JSON types (internal) ---

#[derive(Deserialize)]
struct CargoMessage {
    reason: String,
    message: Option<DiagnosticMessage>,
}

#[derive(Deserialize)]
struct DiagnosticMessage {
    level: String,
    code: Option<DiagnosticCode>,
    message: String,
    spans: Vec<DiagnosticSpan>,
    rendered: Option<String>,
}

#[derive(Deserialize)]
struct DiagnosticCode {
    code: String,
}

#[derive(Deserialize)]
struct DiagnosticSpan {
    file_name: String,
    line_start: usize,
    line_end: usize,
    is_primary: bool,
}

/// Run an integration command and parse its output into findings.
pub fn run_integration(
    skimsystem: &str,
    integration_name: &str,
    spec: &IntegrationSpec,
    root: &Path,
) -> Result<IntegrationReport, IntegrationError> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(&spec.command)
        .current_dir(root)
        .output()
        .map_err(|e| IntegrationError::CommandFailed(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Check for build failure (no JSON output + "could not compile")
    if stdout.is_empty() && stderr.contains("could not compile") {
        return Ok(IntegrationReport {
            skimsystem: skimsystem.to_string(),
            integration_name: integration_name.to_string(),
            total_findings: 0,
            findings_by_subsystem: HashMap::new(),
            unowned_findings: Vec::new(),
            files_written: 0,
            change_requests_generated: 0,
            build_error: Some(stderr.to_string()),
        });
    }

    let findings = match spec.format {
        IntegrationFormat::CargoDiagnostic => parse_cargo_diagnostic(&stdout),
    };

    let total = findings.len();
    let (by_subsystem, unowned) = map_findings_to_subsystems(&findings, root);

    Ok(IntegrationReport {
        skimsystem: skimsystem.to_string(),
        integration_name: integration_name.to_string(),
        total_findings: total,
        findings_by_subsystem: by_subsystem,
        unowned_findings: unowned,
        files_written: 0,
        change_requests_generated: 0,
        build_error: None,
    })
}

/// Parse `cargo clippy --message-format=json` output.
fn parse_cargo_diagnostic(stdout: &str) -> Vec<IntegrationFinding> {
    let mut findings = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let msg: CargoMessage = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if msg.reason != "compiler-message" {
            continue;
        }

        let Some(diag) = msg.message else { continue };

        // Only warnings — errors are build failures, notes are sub-diagnostics
        if diag.level != "warning" {
            continue;
        }

        // Must have a code (summary lines like "N warnings generated" do not)
        let Some(code) = diag.code else { continue };

        // Find primary span
        let Some(span) = diag.spans.iter().find(|s| s.is_primary) else {
            continue;
        };

        findings.push(IntegrationFinding {
            file_path: span.file_name.clone(),
            line_start: span.line_start,
            line_end: span.line_end,
            code: code.code,
            level: FindingLevel::Warning,
            message: diag.message,
            rendered: diag.rendered.unwrap_or_default(),
        });
    }

    findings
}

/// Group findings by their owning subsystem.
fn map_findings_to_subsystems(
    findings: &[IntegrationFinding],
    root: &Path,
) -> (HashMap<String, Vec<IntegrationFinding>>, Vec<IntegrationFinding>) {
    let mut by_file: HashMap<&str, Vec<&IntegrationFinding>> = HashMap::new();
    for f in findings {
        by_file.entry(&f.file_path).or_default().push(f);
    }

    let mut by_subsystem: HashMap<String, Vec<IntegrationFinding>> = HashMap::new();
    let mut unowned = Vec::new();

    for (file_path, file_findings) in by_file {
        let source_path = root.join(file_path);
        match stub::find_subsystem_for_file(&source_path, root) {
            Some((_owner, subsystem_name)) => {
                by_subsystem
                    .entry(subsystem_name)
                    .or_default()
                    .extend(file_findings.into_iter().cloned());
            }
            None => {
                unowned.extend(file_findings.into_iter().cloned());
            }
        }
    }

    (by_subsystem, unowned)
}

/// Find which function encloses a given line number using tree-sitter.
fn find_enclosing_function(file_path: &str, line: usize, root: &Path) -> String {
    let source_path = root.join(file_path);
    let source = match std::fs::read_to_string(&source_path) {
        Ok(s) => s,
        Err(_) => return "unknown".to_string(),
    };
    let symbols = match treesitter::extract_symbols(&source) {
        Ok(s) => s,
        Err(_) => return "unknown".to_string(),
    };
    for sym in &symbols {
        if line >= sym.start_line && line <= sym.end_line {
            return sym.name.clone();
        }
    }
    "file".to_string()
}

/// Generate a deterministic ID for a finding (for deduplication across re-runs).
fn generate_finding_id(skimsystem: &str, integration: &str, finding: &IntegrationFinding) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    finding.file_path.hash(&mut hasher);
    finding.code.hash(&mut hasher);
    finding.line_start.hash(&mut hasher);
    let hash = hasher.finish();
    format!("{skimsystem}-{integration}-{hash:016x}")
}

/// Write integration results (skim observations + change_requests) to .bog sidecar files.
pub fn write_integration_results(
    skimsystem: &str,
    integration_name: &str,
    owner: &str,
    report: &mut IntegrationReport,
    root: &Path,
) -> Result<(), IntegrationError> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let marker = format!("// [integration:{skimsystem}:{integration_name}]");

    // Group all findings by file path
    let mut by_file: HashMap<String, Vec<&IntegrationFinding>> = HashMap::new();
    for findings in report.findings_by_subsystem.values() {
        for f in findings {
            by_file.entry(f.file_path.clone()).or_default().push(f);
        }
    }

    for (file_path, findings) in &by_file {
        let bog_path = root.join(format!("{file_path}.bog"));
        let mut content = if bog_path.exists() {
            std::fs::read_to_string(&bog_path).unwrap_or_default()
        } else {
            let source_path = root.join(file_path);
            stub::generate_file_header(&source_path, root)
        };

        // Remove previous integration section (from marker to next marker or EOF)
        if let Some(marker_pos) = content.find(&marker) {
            // Look for the next marker after this one
            let after_marker = marker_pos + marker.len();
            if let Some(next_marker) = content[after_marker..].find("\n// [integration:") {
                // Truncate just this section, keep the rest
                let keep_after = after_marker + next_marker;
                let before = &content[..marker_pos];
                let after = &content[keep_after..];
                content = format!("{before}{after}");
            } else {
                content.truncate(marker_pos);
            }
        }

        // Ensure trailing newline
        if !content.ends_with('\n') {
            content.push('\n');
        }

        // Write marker
        content.push_str(&format!("\n{marker}\n"));

        // Write skim observation
        let skim_status = if findings.len() > 5 {
            "red"
        } else if findings.is_empty() {
            "green"
        } else {
            "yellow"
        };
        content.push_str(&format!(
            "#[skim({skimsystem}) {{\n  status = {skim_status},\n  notes = \"{integration_name}: {} warning(s)\"\n}}]\n",
            findings.len()
        ));

        // Write change_requests block
        if !findings.is_empty() {
            content.push_str("\n#[change_requests {\n");
            for finding in findings {
                let id = generate_finding_id(skimsystem, integration_name, finding);
                let target_fn = find_enclosing_function(&finding.file_path, finding.line_start, root);
                let target_str = if target_fn == "file" {
                    "file".to_string()
                } else {
                    format!("fn({target_fn})")
                };
                let desc = finding.message.replace('"', "\\\"");
                content.push_str(&format!(
                    "  #[request(\n    id = \"{id}\",\n    from = \"{owner}\",\n    target = {target_str},\n    type = lint_warning,\n    status = pending,\n    created = \"{today}\",\n    description = \"{} (line {}): {desc}\"\n  )]\n",
                    finding.code, finding.line_start
                ));
                report.change_requests_generated += 1;
            }
            content.push_str("}]\n");
        }

        std::fs::write(&bog_path, &content).map_err(|e| {
            IntegrationError::WriteFailed(bog_path.display().to_string(), e.to_string())
        })?;
        report.files_written += 1;
    }

    Ok(())
}

/// Print a summary of findings grouped by subsystem.
pub fn print_report(report: &IntegrationReport) {
    if let Some(err) = &report.build_error {
        println!("  {} Build failed:\n{err}", "error:".red());
        return;
    }

    println!("  Found {} warning(s)", report.total_findings);

    for (subsystem, findings) in &report.findings_by_subsystem {
        println!(
            "    {} {subsystem}: {} finding(s)",
            ">>".dimmed(),
            findings.len()
        );
    }

    if !report.unowned_findings.is_empty() {
        println!(
            "  {} {} finding(s) in files not belonging to any subsystem",
            "warn:".yellow(),
            report.unowned_findings.len()
        );
    }

    if report.change_requests_generated > 0 {
        println!(
            "  {} Wrote {} change request(s) across {} file(s)",
            "ok:".green(),
            report.change_requests_generated,
            report.files_written
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cargo_diagnostic_basic() {
        let json = r#"{"reason":"compiler-message","package_id":"bog","manifest_path":"Cargo.toml","message":{"rendered":"warning: foo","message":"this argument is passed by value, but not consumed","code":{"code":"clippy::needless_pass_by_value"},"level":"warning","spans":[{"file_name":"src/parser.rs","byte_start":100,"byte_end":200,"line_start":42,"line_end":42,"column_start":1,"column_end":10,"is_primary":true,"text":[]}],"children":[]}}"#;
        let findings = parse_cargo_diagnostic(json);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].file_path, "src/parser.rs");
        assert_eq!(findings[0].line_start, 42);
        assert_eq!(findings[0].code, "clippy::needless_pass_by_value");
        assert_eq!(findings[0].level, FindingLevel::Warning);
    }

    #[test]
    fn test_parse_cargo_diagnostic_skips_non_warnings() {
        let error_json = r#"{"reason":"compiler-message","package_id":"bog","manifest_path":"Cargo.toml","message":{"rendered":"error: foo","message":"cannot find","code":{"code":"E0425"},"level":"error","spans":[{"file_name":"src/foo.rs","byte_start":0,"byte_end":1,"line_start":1,"line_end":1,"column_start":1,"column_end":2,"is_primary":true,"text":[]}],"children":[]}}"#;
        let build_json = r#"{"reason":"build-script-executed","package_id":"foo","out_dir":"/tmp"}"#;
        let input = format!("{error_json}\n{build_json}");
        let findings = parse_cargo_diagnostic(&input);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_parse_cargo_diagnostic_skips_no_code() {
        // Summary line: "warning: 5 warnings emitted" has no code
        let json = r#"{"reason":"compiler-message","package_id":"bog","manifest_path":"Cargo.toml","message":{"rendered":"warning: 5 warnings emitted","message":"5 warnings emitted","code":null,"level":"warning","spans":[],"children":[]}}"#;
        let findings = parse_cargo_diagnostic(json);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_generate_finding_id_deterministic() {
        let f = IntegrationFinding {
            file_path: "src/foo.rs".to_string(),
            line_start: 42,
            line_end: 42,
            code: "clippy::test".to_string(),
            level: FindingLevel::Warning,
            message: "test".to_string(),
            rendered: String::new(),
        };
        let id1 = generate_finding_id("sk", "int", &f);
        let id2 = generate_finding_id("sk", "int", &f);
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_generate_finding_id_varies() {
        let f1 = IntegrationFinding {
            file_path: "src/foo.rs".to_string(),
            line_start: 42,
            line_end: 42,
            code: "clippy::a".to_string(),
            level: FindingLevel::Warning,
            message: "a".to_string(),
            rendered: String::new(),
        };
        let f2 = IntegrationFinding {
            file_path: "src/foo.rs".to_string(),
            line_start: 99,
            line_end: 99,
            code: "clippy::b".to_string(),
            level: FindingLevel::Warning,
            message: "b".to_string(),
            rendered: String::new(),
        };
        assert_ne!(
            generate_finding_id("sk", "int", &f1),
            generate_finding_id("sk", "int", &f2)
        );
    }
}
