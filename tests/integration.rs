use std::path::Path;

use bogbot::config;
use bogbot::health;
use bogbot::parser;
use bogbot::treesitter;
use bogbot::validator;

const FIXTURES: &str = "tests/fixtures";

#[test]
fn test_config_loading() {
    let config = config::load_config(Path::new(FIXTURES).join("bogbot.toml").as_path()).unwrap();
    assert_eq!(config.bogbot.version, "0.1.0");
    assert!(config.agents.contains_key("auth-agent"));
    assert!(config.agents.contains_key("db-agent"));
    assert_eq!(config.tree_sitter.language, "rust");
    assert_eq!(config.health.dimensions.len(), 3);
}

#[test]
fn test_repo_bog_parsing() {
    let content = std::fs::read_to_string(Path::new(FIXTURES).join("repo.bog")).unwrap();
    let bog = parser::parse_bog(&content).unwrap();
    assert_eq!(bog.annotations.len(), 3);
}

#[test]
fn test_file_bog_parsing() {
    let content =
        std::fs::read_to_string(Path::new(FIXTURES).join("src/auth.rs.bog")).unwrap();
    let bog = parser::parse_bog(&content).unwrap();
    // file, description, health, fn(login), fn(logout)
    assert_eq!(bog.annotations.len(), 5);
}

#[test]
fn test_treesitter_validates_functions() {
    let source = std::fs::read_to_string(Path::new(FIXTURES).join("src/auth.rs")).unwrap();
    let symbols = treesitter::extract_symbols(&source).unwrap();
    let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"login"));
    assert!(names.contains(&"logout"));
}

#[test]
fn test_validate_functions_match() {
    let bog_path = Path::new(FIXTURES).join("src/auth.rs.bog");
    let source_path = Path::new(FIXTURES).join("src/auth.rs");
    let bog = validator::validate_syntax(&bog_path).unwrap();
    let errors = validator::validate_functions(&bog_path, &bog, &source_path);
    assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
}

#[test]
fn test_validate_functions_catches_mismatch() {
    // Parse a .bog file that references a non-existent function
    let input = r#"
#[file(
  owner = "auth-agent",
  subsystem = "authentication",
  updated = "2026-02-18",
  status = green
)]

#[fn(nonexistent_function) {
  status = red
}]
"#;
    let bog = parser::parse_bog(input).unwrap();
    let source_path = Path::new(FIXTURES).join("src/auth.rs");
    let bog_path = Path::new("test.rs.bog");
    let errors = validator::validate_functions(bog_path, &bog, &source_path);
    assert_eq!(errors.len(), 1);
    match &errors[0] {
        validator::ValidationError::MissingFunction { function, .. } => {
            assert_eq!(function, "nonexistent_function");
        }
        _ => panic!("expected MissingFunction error"),
    }
}

#[test]
fn test_full_project_validation() {
    let report = validator::validate_project(Path::new(FIXTURES));
    for e in &report.errors {
        eprintln!("  error: {e}");
    }
    for w in &report.warnings {
        eprintln!("  warn: {w}");
    }
    assert!(report.is_ok(), "Expected validation to pass");
    assert!(report.files_checked >= 2);
}

#[test]
fn test_health_report() {
    let health = health::compute_health(Path::new(FIXTURES));
    assert_eq!(health.name, "test-project");
    assert!(!health.subsystems.is_empty());

    let auth = &health.subsystems[0];
    assert_eq!(auth.name, "authentication");
    assert_eq!(auth.owner, "auth-agent");
}
