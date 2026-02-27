use std::path::{Path, PathBuf};

use bog::ast::{Annotation, IntegrationFormat};
use bog::config;
use bog::context;
use bog::health;
use bog::parser;
use bog::treesitter;
use bog::validator;

/// Resolve the project root from CARGO_MANIFEST_DIR.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

// --- Config ---

#[test]
fn test_config_loading() {
    let root = workspace_root();
    let config = config::load_config(&root.join("bog.toml")).unwrap();
    assert_eq!(config.bog.version, "0.1.0");
    assert_eq!(config.tree_sitter.language, "rust");
    assert_eq!(config.health.dimensions.len(), 4);
}

#[test]
fn test_derive_agents_from_repo_bog() {
    use bog::ast;
    use bog::config::AgentRole;

    let root = workspace_root();
    let content = std::fs::read_to_string(root.join("repo.bog")).unwrap();
    let bog = parser::parse_bog(&content).unwrap();
    let derived = ast::derive_agents(&bog);

    assert!(derived.roles.contains_key("core-agent"));
    assert!(derived.roles.contains_key("analysis-agent"));
    assert!(derived.roles.contains_key("cli-agent"));
    assert!(derived.roles.contains_key("orchestrate-agent"));
    assert!(derived.roles.contains_key("code-standards-agent"));
    assert!(derived.roles.contains_key("bog-health-agent"));
    assert!(derived.roles.contains_key("observability-agent"));

    assert_eq!(derived.roles["core-agent"], AgentRole::Subsystem);
    assert_eq!(derived.roles["orchestrate-agent"], AgentRole::Subsystem);
    assert_eq!(derived.roles["code-standards-agent"], AgentRole::Skimsystem);
    assert_eq!(derived.roles["observability-agent"], AgentRole::Skimsystem);
}

// --- Parser ---

#[test]
fn test_repo_bog_parsing() {
    let root = workspace_root();
    let content = std::fs::read_to_string(root.join("repo.bog")).unwrap();
    let bog = parser::parse_bog(&content).unwrap();
    // repo, description, 4 subsystems, 1 skimsystem, policies
    assert!(bog.annotations.len() >= 8);
}

#[test]
fn test_file_bog_parsing() {
    let root = workspace_root();
    let content = std::fs::read_to_string(root.join("src/parser.rs.bog")).unwrap();
    let bog = parser::parse_bog(&content).unwrap();
    // file, description, health, plus many fn annotations
    assert!(bog.annotations.len() >= 4);
}

#[test]
fn test_fixture_bog_parsing() {
    let root = workspace_root();
    let content = std::fs::read_to_string(root.join("tests/fixtures/src/auth.rs.bog")).unwrap();
    let bog = parser::parse_bog(&content).unwrap();
    // file, description, health, skim(tracing), fn(login), fn(logout)
    assert_eq!(bog.annotations.len(), 6);
}

// --- Tree-sitter ---

#[test]
fn test_treesitter_validates_functions() {
    let root = workspace_root();
    let source = std::fs::read_to_string(root.join("tests/fixtures/src/auth.rs")).unwrap();
    let symbols = treesitter::extract_symbols(&source).unwrap();
    let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"login"));
    assert!(names.contains(&"logout"));
}

// --- Validator ---

#[test]
fn test_validate_functions_match() {
    let root = workspace_root();
    let bog_path = root.join("tests/fixtures/src/auth.rs.bog");
    let source_path = root.join("tests/fixtures/src/auth.rs");
    let bog = validator::validate_syntax(&bog_path).unwrap();
    let errors = validator::validate_functions(&bog_path, &bog, &source_path);
    assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
}

#[test]
fn test_validate_functions_catches_mismatch() {
    let input = r#"
#[file(
  owner = "analysis-agent",
  subsystem = "test-fixtures",
  updated = "2026-02-24",
  status = green
)]

#[fn(nonexistent_function) {
  status = red
}]
"#;
    let bog = parser::parse_bog(input).unwrap();
    let root = workspace_root();
    let source_path = root.join("tests/fixtures/src/auth.rs");
    let bog_path = root.join("test.rs.bog");
    let errors = validator::validate_functions(&bog_path, &bog, &source_path);
    assert_eq!(errors.len(), 1);
    match &errors[0] {
        validator::ValidationError::MissingFunction { function, .. } => {
            assert_eq!(function, "nonexistent_function");
        }
        _ => panic!("expected MissingFunction error"),
    }
}

// --- Dogfood: bog validates itself ---

#[test]
fn test_dogfood_validate() {
    let root = workspace_root();
    let report = validator::validate_project(&root);
    for e in &report.errors {
        eprintln!("  dogfood error: {e}");
    }
    for w in &report.warnings {
        eprintln!("  dogfood warn: {w}");
    }
    assert!(
        report.is_ok(),
        "bog must validate its own .bog annotations"
    );
    assert!(
        report.files_checked >= 25,
        "expected at least 25 .bog files, got {}",
        report.files_checked
    );
}

#[test]
fn test_dogfood_health() {
    let root = workspace_root();
    let health = health::compute_health(&root);
    assert_eq!(health.name, "bog");
    assert_eq!(health.subsystems.len(), 5);

    let names: Vec<&str> = health
        .subsystems
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert!(names.contains(&"core"));
    assert!(names.contains(&"analysis"));
    assert!(names.contains(&"cli"));
    assert!(names.contains(&"orchestrate"));
    assert!(names.contains(&"test-fixtures"));
}

#[test]
fn test_dogfood_skimsystem_declared() {
    let root = workspace_root();
    let content = std::fs::read_to_string(root.join("repo.bog")).unwrap();
    let bog = parser::parse_bog(&content).unwrap();
    let skimsystems: Vec<_> = bog
        .annotations
        .iter()
        .filter(|a| matches!(a, Annotation::Skimsystem(_)))
        .collect();
    assert!(
        !skimsystems.is_empty(),
        "bog should declare at least one skimsystem"
    );
}

#[test]
fn test_dogfood_skim_observations_valid() {
    let root = workspace_root();
    let report = validator::validate_project(&root);
    let skim_errors: Vec<_> = report
        .errors
        .iter()
        .filter(|e| {
            matches!(
                e,
                validator::ValidationError::UndeclaredSkimsystem { .. }
                    | validator::ValidationError::SkimTargetFunctionMissing { .. }
            )
        })
        .collect();
    assert!(
        skim_errors.is_empty(),
        "all skim observations should reference valid skimsystems: {skim_errors:?}"
    );
}

#[test]
fn test_dogfood_skimsystem_health() {
    let root = workspace_root();
    let health = health::compute_health(&root);
    assert!(
        !health.skimsystems.is_empty(),
        "should have skimsystem health"
    );
    let aq = health
        .skimsystems
        .iter()
        .find(|s| s.name == "annotation-quality");
    assert!(aq.is_some(), "should have annotation-quality skimsystem");
    assert!(
        aq.unwrap().observation_count > 0,
        "annotation-quality should have observations"
    );
}

#[test]
fn test_dogfood_code_quality_integration() {
    let root = workspace_root();
    let content = std::fs::read_to_string(root.join("repo.bog")).unwrap();
    let bog = parser::parse_bog(&content).unwrap();
    let cq = bog.annotations.iter().find_map(|a| {
        if let Annotation::Skimsystem(sk) = a {
            if sk.name == "code-quality" {
                return Some(sk);
            }
        }
        None
    });
    assert!(cq.is_some(), "should have code-quality skimsystem");
    let cq = cq.unwrap();
    assert_eq!(cq.integrations.len(), 1);
    assert_eq!(cq.integrations[0].name, "clippy");
    assert_eq!(cq.integrations[0].format, IntegrationFormat::CargoDiagnostic);
    assert!(cq.integrations[0].command.contains("clippy"));
}

// --- Context ---

#[test]
fn test_context_all_scope() {
    let root = workspace_root();
    let output = context::load_context(
        &root,
        context::ContextScope::All,
        context::SectionFilter::all(),
        None,
        None,
    )
    .unwrap();
    assert_eq!(output.scope.kind, "all");
    assert!(!output.files.is_empty(), "should have annotated files");
    assert!(
        output.scope.subsystems.len() >= 4,
        "should have at least 4 subsystems"
    );
}

#[test]
fn test_context_agent_scope() {
    let root = workspace_root();
    let output = context::load_context(
        &root,
        context::ContextScope::Agent("core-agent".to_string()),
        context::SectionFilter::all(),
        None,
        None,
    )
    .unwrap();
    assert_eq!(output.scope.kind, "agent");
    assert_eq!(output.scope.name.as_deref(), Some("core-agent"));
    assert_eq!(output.scope.subsystems.len(), 1);
    assert_eq!(output.scope.subsystems[0].name, "core");

    let paths: Vec<&str> = output.files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.iter().any(|p| p.contains("ast.rs")));
    assert!(paths.iter().any(|p| p.contains("parser.rs")));
    assert!(paths.iter().any(|p| p.contains("config.rs")));
    assert!(!paths.iter().any(|p| p.contains("cli.rs")));
    assert!(!paths.iter().any(|p| p.contains("validator.rs")));
}

#[test]
fn test_context_subsystem_scope() {
    let root = workspace_root();
    let output = context::load_context(
        &root,
        context::ContextScope::Subsystem("cli".to_string()),
        context::SectionFilter::all(),
        None,
        None,
    )
    .unwrap();
    assert_eq!(output.scope.kind, "subsystem");
    assert_eq!(output.scope.name.as_deref(), Some("cli"));

    let paths: Vec<&str> = output.files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.iter().any(|p| p.contains("cli.rs")));
    assert!(paths.iter().any(|p| p.contains("main.rs")));
}

#[test]
fn test_context_unknown_agent_error() {
    let root = workspace_root();
    let result = context::load_context(
        &root,
        context::ContextScope::Agent("ghost-agent".to_string()),
        context::SectionFilter::all(),
        None,
        None,
    );
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("ghost-agent"),
        "error should mention the agent name"
    );
}

#[test]
fn test_context_section_filter() {
    let root = workspace_root();
    let filter = context::SectionFilter {
        pickled: false,
        requests: true,
        health: false,
        contracts: false,
        skims: false,
    };
    let output = context::load_context(
        &root,
        context::ContextScope::All,
        filter,
        None,
        None,
    )
    .unwrap();

    for file in &output.files {
        assert!(file.health.is_none(), "health should be filtered out");
        assert!(file.pickled.is_empty(), "pickled should be filtered out");
        assert!(file.fn_contracts.is_empty(), "contracts should be filtered out");
        assert!(file.skim_observations.is_empty(), "skims should be filtered out");
    }
}

#[test]
fn test_context_json_serializable() {
    let root = workspace_root();
    let output = context::load_context(
        &root,
        context::ContextScope::Agent("core-agent".to_string()),
        context::SectionFilter::all(),
        None,
        None,
    )
    .unwrap();
    let json = serde_json::to_string(&output).expect("should serialize to JSON");
    assert!(json.contains("\"scope\""));
    assert!(json.contains("\"files\""));
    assert!(json.contains("core-agent"));
}

#[test]
fn test_dogfood_every_subsystem_has_files() {
    let root = workspace_root();
    let health = health::compute_health(&root);
    for sub in &health.subsystems {
        assert!(
            sub.file_count > 0,
            "subsystem '{}' has no annotated files",
            sub.name
        );
    }
}
