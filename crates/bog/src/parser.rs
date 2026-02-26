use std::collections::HashMap;

use pest::iterators::{Pair, Pairs};
use pest::Parser;
use pest_derive::Parser;

use crate::ast::*;

#[derive(Parser)]
#[grammar = "parser.pest"]
pub struct BogParser;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("Parse error: {0}")]
    Pest(#[from] pest::error::Error<Rule>),

    #[error("Unknown annotation type: {0}")]
    UnknownAnnotation(String),

    #[error("Missing required field '{field}' in {context}")]
    MissingField { context: String, field: String },

    #[error("Invalid value for field '{field}': {message}")]
    InvalidValue { field: String, message: String },
}

pub fn parse_bog(input: &str) -> Result<BogFile, ParseError> {
    let mut pairs = BogParser::parse(Rule::bog_file, input)?;
    let bog_file = pairs.next().unwrap();
    let mut annotations = Vec::new();

    for pair in bog_file.into_inner() {
        if pair.as_rule() == Rule::annotation {
            annotations.push(parse_annotation(pair)?);
        }
    }

    Ok(BogFile { annotations })
}

fn parse_annotation(pair: Pair<Rule>) -> Result<Annotation, ParseError> {
    let mut inner = pair.into_inner();
    let ident_pair = inner.next().unwrap();
    let name = ident_pair.as_str();

    match name {
        "repo" => parse_repo(inner),
        "file" => parse_file(inner),
        "description" => parse_description(inner),
        "health" => parse_health(inner),
        "fn" => parse_fn(inner),
        "subsystem" => parse_subsystem(inner),
        "skimsystem" => parse_skimsystem(inner),
        "skim" => parse_skim(inner),
        "policies" => parse_policies(inner),
        "change_requests" => parse_change_requests(inner),
        "pickled" => parse_pickled(inner),
        other => Err(ParseError::UnknownAnnotation(other.to_string())),
    }
}

// --- Helper functions ---

fn extract_kv_map(pairs: Pairs<Rule>) -> Result<HashMap<String, Value>, ParseError> {
    let mut map = HashMap::new();
    for pair in pairs {
        if pair.as_rule() == Rule::kv_pair {
            let mut kv_inner = pair.into_inner();
            let key = kv_inner.next().unwrap().as_str().to_string();
            let val = parse_value(kv_inner.next().unwrap())?;
            map.insert(key, val);
        }
    }
    Ok(map)
}

fn get_kv_list_from_parens(pairs: &mut Pairs<Rule>) -> Result<HashMap<String, Value>, ParseError> {
    if let Some(parens) = pairs.next() {
        if parens.as_rule() == Rule::parens {
            let content = parens.into_inner().next().unwrap();
            if content.as_rule() == Rule::parens_content {
                let inner = content.into_inner().next().unwrap();
                if inner.as_rule() == Rule::kv_list {
                    return extract_kv_map(inner.into_inner());
                }
            }
        }
    }
    Ok(HashMap::new())
}

fn get_ident_from_parens(pairs: &mut Pairs<Rule>) -> Option<String> {
    if let Some(parens) = pairs.next() {
        if parens.as_rule() == Rule::parens {
            let content = parens.into_inner().next().unwrap();
            if content.as_rule() == Rule::parens_content {
                let inner = content.into_inner().next().unwrap();
                if inner.as_rule() == Rule::ident {
                    return Some(inner.as_str().to_string());
                }
            }
        }
    }
    None
}

fn get_body_kv_map(pairs: &mut Pairs<Rule>) -> Result<HashMap<String, Value>, ParseError> {
    if let Some(body) = pairs.next() {
        if body.as_rule() == Rule::body {
            if let Some(content) = body.into_inner().next() {
                if content.as_rule() == Rule::body_content {
                    let inner = content.into_inner().next().unwrap();
                    if inner.as_rule() == Rule::kv_list {
                        return extract_kv_map(inner.into_inner());
                    }
                }
            }
        }
    }
    Ok(HashMap::new())
}


fn get_body_text(pairs: &mut Pairs<Rule>) -> String {
    if let Some(body) = pairs.next() {
        if body.as_rule() == Rule::body {
            if let Some(content) = body.into_inner().next() {
                // Use raw text regardless of how pest parsed it
                return content.as_str().trim().to_string();
            }
        }
    }
    String::new()
}

fn require_string(map: &HashMap<String, Value>, key: &str, ctx: &str) -> Result<String, ParseError> {
    match map.get(key) {
        Some(Value::String(s)) => Ok(unquote(s)),
        Some(Value::Ident(s)) => Ok(s.clone()),
        Some(_) => Err(ParseError::InvalidValue {
            field: key.to_string(),
            message: format!("expected string in {ctx}"),
        }),
        None => Err(ParseError::MissingField {
            context: ctx.to_string(),
            field: key.to_string(),
        }),
    }
}

fn require_status(map: &HashMap<String, Value>, key: &str, ctx: &str) -> Result<Status, ParseError> {
    match map.get(key) {
        Some(Value::Status(s)) => Ok(*s),
        Some(_) => Err(ParseError::InvalidValue {
            field: key.to_string(),
            message: format!("expected status (green/yellow/red) in {ctx}"),
        }),
        None => Err(ParseError::MissingField {
            context: ctx.to_string(),
            field: key.to_string(),
        }),
    }
}

fn opt_string(map: &HashMap<String, Value>, key: &str) -> Option<String> {
    match map.get(key) {
        Some(Value::String(s)) => Some(unquote(s)),
        Some(Value::Ident(s)) => Some(s.clone()),
        _ => None,
    }
}

fn extract_string_list(map: &HashMap<String, Value>, key: &str) -> Vec<String> {
    match map.get(key) {
        Some(Value::List(items)) => items
            .iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(unquote(s)),
                Value::Ident(s) => Some(s.clone()),
                Value::Path(parts) => Some(parts.join("::")),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn unquote(s: &str) -> String {
    let s = s.strip_prefix('"').unwrap_or(s);
    let s = s.strip_suffix('"').unwrap_or(s);
    s.replace("\\\"", "\"").replace("\\\\", "\\")
}

// --- Value parsing ---

fn parse_value(pair: Pair<Rule>) -> Result<Value, ParseError> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::string_literal => Ok(Value::String(inner.as_str().to_string())),
        Rule::status_literal => {
            let status = match inner.as_str() {
                "green" => Status::Green,
                "yellow" => Status::Yellow,
                "red" => Status::Red,
                _ => unreachable!(),
            };
            Ok(Value::Status(status))
        }
        Rule::bool_literal => Ok(Value::Bool(inner.as_str() == "true")),
        Rule::number_literal => {
            let n: i64 = inner.as_str().parse().map_err(|_| ParseError::InvalidValue {
                field: "number".to_string(),
                message: format!("invalid number: {}", inner.as_str()),
            })?;
            Ok(Value::Number(n))
        }
        Rule::fn_ref => {
            // fn_ref children are: fn_keyword, ident — skip fn_keyword
            let mut fn_inner = inner.into_inner();
            fn_inner.next(); // skip fn_keyword
            let name = fn_inner.next().unwrap().as_str().to_string();
            Ok(Value::FnRef(name))
        }
        Rule::ident_path => {
            let parts: Vec<String> = inner.as_str().split("::").map(|s| s.to_string()).collect();
            Ok(Value::Path(parts))
        }
        Rule::ident => Ok(Value::Ident(inner.as_str().to_string())),
        Rule::list => {
            let items: Result<Vec<Value>, _> = inner
                .into_inner()
                .map(|item| {
                    let inner_val = item.into_inner().next().unwrap();
                    parse_list_item_value(inner_val)
                })
                .collect();
            Ok(Value::List(items?))
        }
        Rule::tuple => {
            let items: Result<Vec<Value>, ParseError> = inner
                .into_inner()
                .map(|item| -> Result<Value, ParseError> {
                    let inner_val = item.into_inner().next().unwrap();
                    match inner_val.as_rule() {
                        Rule::string_literal => Ok(Value::String(inner_val.as_str().to_string())),
                        Rule::ident => Ok(Value::Ident(inner_val.as_str().to_string())),
                        _ => Ok(Value::Ident(inner_val.as_str().to_string())),
                    }
                })
                .collect();
            Ok(Value::Tuple(items?))
        }
        Rule::nested_block => {
            let kv_map = extract_kv_map(inner.into_inner().next().unwrap().into_inner())?;
            let pairs: Vec<(String, Value)> = kv_map.into_iter().collect();
            Ok(Value::Block(pairs))
        }
        _ => Ok(Value::Ident(inner.as_str().to_string())),
    }
}

fn parse_list_item_value(pair: Pair<Rule>) -> Result<Value, ParseError> {
    match pair.as_rule() {
        Rule::string_literal => Ok(Value::String(pair.as_str().to_string())),
        Rule::status_literal => {
            let status = match pair.as_str() {
                "green" => Status::Green,
                "yellow" => Status::Yellow,
                "red" => Status::Red,
                _ => unreachable!(),
            };
            Ok(Value::Status(status))
        }
        Rule::bool_literal => Ok(Value::Bool(pair.as_str() == "true")),
        Rule::number_literal => {
            let n: i64 = pair.as_str().parse().map_err(|_| ParseError::InvalidValue {
                field: "number".to_string(),
                message: format!("invalid number: {}", pair.as_str()),
            })?;
            Ok(Value::Number(n))
        }
        Rule::fn_ref => {
            let name = pair.into_inner().next().unwrap().as_str().to_string();
            Ok(Value::FnRef(name))
        }
        Rule::ident_path => {
            let parts: Vec<String> = pair.as_str().split("::").map(|s| s.to_string()).collect();
            Ok(Value::Path(parts))
        }
        Rule::ident => Ok(Value::Ident(pair.as_str().to_string())),
        Rule::tuple => {
            let items: Result<Vec<Value>, ParseError> = pair
                .into_inner()
                .map(|item| -> Result<Value, ParseError> {
                    let inner_val = item.into_inner().next().unwrap();
                    match inner_val.as_rule() {
                        Rule::string_literal => Ok(Value::String(inner_val.as_str().to_string())),
                        Rule::ident => Ok(Value::Ident(inner_val.as_str().to_string())),
                        _ => Ok(Value::Ident(inner_val.as_str().to_string())),
                    }
                })
                .collect();
            Ok(Value::Tuple(items?))
        }
        _ => Ok(Value::Ident(pair.as_str().to_string())),
    }
}

// --- Annotation type parsers ---

fn parse_repo(mut pairs: Pairs<Rule>) -> Result<Annotation, ParseError> {
    let map = get_kv_list_from_parens(&mut pairs)?;
    Ok(Annotation::Repo(RepoAnnotation {
        name: require_string(&map, "name", "repo")?,
        version: require_string(&map, "version", "repo")?,
        updated: require_string(&map, "updated", "repo")?,
    }))
}

fn parse_file(mut pairs: Pairs<Rule>) -> Result<Annotation, ParseError> {
    let map = get_kv_list_from_parens(&mut pairs)?;
    Ok(Annotation::File(FileAnnotation {
        owner: require_string(&map, "owner", "file")?,
        subsystem: require_string(&map, "subsystem", "file")?,
        updated: require_string(&map, "updated", "file")?,
        status: require_status(&map, "status", "file")?,
    }))
}

fn parse_description(mut pairs: Pairs<Rule>) -> Result<Annotation, ParseError> {
    let text = get_body_text(&mut pairs);
    Ok(Annotation::Description(text))
}

fn parse_health(mut pairs: Pairs<Rule>) -> Result<Annotation, ParseError> {
    let map = get_kv_list_from_parens(&mut pairs)?;
    let mut dimensions = HashMap::new();
    for (key, val) in map {
        if let Value::Status(s) = val {
            dimensions.insert(key, s);
        }
    }
    Ok(Annotation::Health(HealthAnnotation { dimensions }))
}

fn parse_fn(mut pairs: Pairs<Rule>) -> Result<Annotation, ParseError> {
    let name = get_ident_from_parens(&mut pairs)
        .ok_or_else(|| ParseError::MissingField {
            context: "fn".to_string(),
            field: "name".to_string(),
        })?;
    let map = get_body_kv_map(&mut pairs)?;

    let contract = if let Some(Value::Block(block_pairs)) = map.get("contract") {
        let block_map: HashMap<String, Value> = block_pairs.iter().cloned().collect();
        let inputs = match block_map.get("in") {
            Some(Value::List(items)) => items
                .iter()
                .filter_map(|v| {
                    if let Value::Tuple(parts) = v {
                        if parts.len() == 2 {
                            let name = match &parts[0] {
                                Value::Ident(s) => s.clone(),
                                Value::String(s) => unquote(s),
                                _ => return None,
                            };
                            let ty = match &parts[1] {
                                Value::Ident(s) => s.clone(),
                                Value::String(s) => unquote(s),
                                _ => return None,
                            };
                            return Some((name, ty));
                        }
                    }
                    None
                })
                .collect(),
            _ => Vec::new(),
        };
        let output = match block_map.get("out") {
            Some(Value::String(s)) => Some(unquote(s)),
            _ => None,
        };
        let invariants = match block_map.get("invariants") {
            Some(Value::List(items)) => items
                .iter()
                .filter_map(|v| {
                    if let Value::String(s) = v {
                        Some(unquote(s))
                    } else {
                        None
                    }
                })
                .collect(),
            _ => Vec::new(),
        };
        Some(Contract { inputs, output, invariants })
    } else {
        None
    };

    let stub = matches!(map.get("stub"), Some(Value::Bool(true)));

    Ok(Annotation::Fn(FnAnnotation {
        name,
        status: require_status(&map, "status", "fn")?,
        stub,
        deps: extract_string_list(&map, "deps"),
        refs: extract_string_list(&map, "refs"),
        contract,
        description: opt_string(&map, "description"),
    }))
}

fn parse_subsystem(mut pairs: Pairs<Rule>) -> Result<Annotation, ParseError> {
    let name = get_ident_from_parens(&mut pairs)
        .ok_or_else(|| ParseError::MissingField {
            context: "subsystem".to_string(),
            field: "name".to_string(),
        })?;
    let map = get_body_kv_map(&mut pairs)?;

    Ok(Annotation::Subsystem(SubsystemDecl {
        name,
        owner: require_string(&map, "owner", "subsystem")?,
        files: extract_string_list(&map, "files"),
        status: require_status(&map, "status", "subsystem")?,
        description: opt_string(&map, "description"),
    }))
}

fn parse_skimsystem(mut pairs: Pairs<Rule>) -> Result<Annotation, ParseError> {
    let name = get_ident_from_parens(&mut pairs)
        .ok_or_else(|| ParseError::MissingField {
            context: "skimsystem".to_string(),
            field: "name".to_string(),
        })?;
    let map = get_body_kv_map(&mut pairs)?;

    let targets = match map.get("targets") {
        Some(Value::Ident(s)) if s == "all" => SkimTargets::All,
        Some(Value::List(items)) => {
            let names = items
                .iter()
                .filter_map(|v| match v {
                    Value::Ident(s) => Some(s.clone()),
                    Value::String(s) => Some(unquote(s)),
                    _ => None,
                })
                .collect();
            SkimTargets::Named(names)
        }
        _ => SkimTargets::All,
    };

    let integrations = if let Some(Value::Block(block_pairs)) = map.get("integrations") {
        let mut specs = Vec::new();
        for (int_name, int_val) in block_pairs {
            if let Value::Block(inner_pairs) = int_val {
                let inner_map: HashMap<String, Value> = inner_pairs.iter().cloned().collect();
                let command = match inner_map.get("command") {
                    Some(Value::String(s)) => unquote(s),
                    _ => {
                        return Err(ParseError::MissingField {
                            context: format!("skimsystem integration '{int_name}'"),
                            field: "command".to_string(),
                        })
                    }
                };
                let format = match inner_map.get("format") {
                    Some(Value::Ident(s)) if s == "cargo_diagnostic" => {
                        IntegrationFormat::CargoDiagnostic
                    }
                    Some(other) => {
                        return Err(ParseError::InvalidValue {
                            field: "format".to_string(),
                            message: format!("unknown integration format: {other:?}"),
                        })
                    }
                    None => {
                        return Err(ParseError::MissingField {
                            context: format!("skimsystem integration '{int_name}'"),
                            field: "format".to_string(),
                        })
                    }
                };
                specs.push(IntegrationSpec {
                    name: int_name.clone(),
                    command,
                    format,
                });
            }
        }
        specs
    } else {
        Vec::new()
    };

    Ok(Annotation::Skimsystem(SkimsystemDecl {
        name,
        owner: require_string(&map, "owner", "skimsystem")?,
        targets,
        status: require_status(&map, "status", "skimsystem")?,
        principles: extract_string_list(&map, "principles"),
        integrations,
        description: opt_string(&map, "description"),
    }))
}

fn parse_skim(mut pairs: Pairs<Rule>) -> Result<Annotation, ParseError> {
    let skimsystem = get_ident_from_parens(&mut pairs)
        .ok_or_else(|| ParseError::MissingField {
            context: "skim".to_string(),
            field: "skimsystem name".to_string(),
        })?;
    let map = get_body_kv_map(&mut pairs)?;

    let target = match map.get("target") {
        Some(Value::FnRef(name)) => Some(SkimTarget::Fn(name.clone())),
        Some(Value::Ident(s)) if s == "file" => Some(SkimTarget::File),
        _ => None,
    };

    Ok(Annotation::Skim(SkimObservation {
        skimsystem,
        status: require_status(&map, "status", "skim")?,
        notes: opt_string(&map, "notes"),
        target,
    }))
}

fn parse_policies(mut pairs: Pairs<Rule>) -> Result<Annotation, ParseError> {
    let map = get_body_kv_map(&mut pairs)?;
    Ok(Annotation::Policies(PoliciesAnnotation { fields: map }))
}

fn parse_change_requests(mut pairs: Pairs<Rule>) -> Result<Annotation, ParseError> {
    let mut requests = Vec::new();
    if let Some(body) = pairs.next() {
        if body.as_rule() == Rule::body {
            if let Some(content) = body.into_inner().next() {
                if content.as_rule() == Rule::body_content {
                    for inner in content.into_inner() {
                        if inner.as_rule() == Rule::annotation {
                            let mut ann_inner = inner.into_inner();
                            let ident = ann_inner.next().unwrap();
                            if ident.as_str() == "request" {
                                let map = get_kv_list_from_parens(&mut ann_inner)?;
                                requests.push(ChangeRequest {
                                    id: require_string(&map, "id", "request")?,
                                    from: require_string(&map, "from", "request")?,
                                    target: map.get("target").cloned().unwrap_or(Value::Ident("unknown".to_string())),
                                    change_type: require_string(&map, "type", "request")?,
                                    status: require_string(&map, "status", "request")?,
                                    priority: opt_string(&map, "priority"),
                                    created: require_string(&map, "created", "request")?,
                                    description: require_string(&map, "description", "request")?,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(Annotation::ChangeRequests(requests))
}

fn parse_pickled(mut pairs: Pairs<Rule>) -> Result<Annotation, ParseError> {
    let parens_map = get_kv_list_from_parens(&mut pairs)?;
    let agent = require_string(&parens_map, "agent", "pickled")?;
    let updated = require_string(&parens_map, "updated", "pickled")?;

    let body_map = get_body_kv_map(&mut pairs)?;
    let id = require_string(&body_map, "id", "pickled")?;
    let kind = match body_map.get("kind") {
        Some(Value::Ident(s)) => match s.as_str() {
            "decision" => PickledKind::Decision,
            "reversal" => PickledKind::Reversal,
            "context" => PickledKind::Context,
            "observation" => PickledKind::Observation,
            "rationale" => PickledKind::Rationale,
            other => return Err(ParseError::InvalidValue {
                field: "kind".to_string(),
                message: format!("unknown pickled kind: {other}"),
            }),
        },
        _ => return Err(ParseError::MissingField {
            context: "pickled".to_string(),
            field: "kind".to_string(),
        }),
    };
    let supersedes = opt_string(&body_map, "supersedes");
    let tags = extract_string_list(&body_map, "tags")
        .iter()
        .map(|s| parse_pickled_tag(s))
        .collect::<Result<Vec<_>, _>>()?;
    let content = require_string(&body_map, "content", "pickled")?;

    Ok(Annotation::Pickled(PickledAnnotation {
        id,
        agent,
        updated,
        kind,
        supersedes,
        tags,
        content,
    }))
}

fn parse_pickled_tag(s: &str) -> Result<PickledTag, ParseError> {
    match s {
        "architecture" => Ok(PickledTag::Architecture),
        "performance" => Ok(PickledTag::Performance),
        "reliability" => Ok(PickledTag::Reliability),
        "security" => Ok(PickledTag::Security),
        "testing" => Ok(PickledTag::Testing),
        "domain" => Ok(PickledTag::Domain),
        "debt" => Ok(PickledTag::Debt),
        "tooling" => Ok(PickledTag::Tooling),
        other => Err(ParseError::InvalidValue {
            field: "tags".to_string(),
            message: format!("unknown pickled tag: {other}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_file_annotation() {
        let input = r#"
#[file(
  owner = "auth-agent",
  subsystem = "authentication",
  updated = "2026-02-18",
  status = green
)]
"#;
        let bog = parse_bog(input).unwrap();
        assert_eq!(bog.annotations.len(), 1);
        match &bog.annotations[0] {
            Annotation::File(f) => {
                assert_eq!(f.owner, "auth-agent");
                assert_eq!(f.subsystem, "authentication");
                assert_eq!(f.status, Status::Green);
            }
            _ => panic!("expected File annotation"),
        }
    }

    #[test]
    fn test_parse_description() {
        let input = r#"
#[description {
  Handles user authentication via JWT.
  Provides login, logout, and token refresh.
}]
"#;
        let bog = parse_bog(input).unwrap();
        assert_eq!(bog.annotations.len(), 1);
        match &bog.annotations[0] {
            Annotation::Description(text) => {
                assert!(text.contains("authentication"));
                assert!(text.contains("JWT"));
            }
            _ => panic!("expected Description annotation"),
        }
    }

    #[test]
    fn test_parse_health() {
        let input = r#"
#[health(
  test_coverage = green,
  staleness = yellow,
  complexity = red
)]
"#;
        let bog = parse_bog(input).unwrap();
        assert_eq!(bog.annotations.len(), 1);
        match &bog.annotations[0] {
            Annotation::Health(h) => {
                assert_eq!(h.dimensions["test_coverage"], Status::Green);
                assert_eq!(h.dimensions["staleness"], Status::Yellow);
                assert_eq!(h.dimensions["complexity"], Status::Red);
            }
            _ => panic!("expected Health annotation"),
        }
    }

    #[test]
    fn test_parse_fn_annotation() {
        let input = r#"
#[fn(login) {
  status = green,
  deps = [db::get_user, crypto::verify_hash],
  refs = [routes::post_login],
  contract = {
    in = [(username, String), (password, String)],
    out = "Result<Token, AuthError>",
    invariants = ["never stores plaintext passwords"]
  },
  description = "Authenticates user, returns JWT token"
}]
"#;
        let bog = parse_bog(input).unwrap();
        assert_eq!(bog.annotations.len(), 1);
        match &bog.annotations[0] {
            Annotation::Fn(f) => {
                assert_eq!(f.name, "login");
                assert_eq!(f.status, Status::Green);
                assert_eq!(f.deps.len(), 2);
                assert_eq!(f.refs.len(), 1);
                let contract = f.contract.as_ref().unwrap();
                assert_eq!(contract.inputs.len(), 2);
                assert_eq!(contract.inputs[0].0, "username");
                assert_eq!(contract.inputs[0].1, "String");
                assert!(contract.output.as_ref().unwrap().contains("Result"));
                assert_eq!(contract.invariants.len(), 1);
            }
            _ => panic!("expected Fn annotation"),
        }
    }

    #[test]
    fn test_parse_repo() {
        let input = r#"
#[repo(
  name = "my-project",
  version = "0.1.0",
  updated = "2026-02-18"
)]
"#;
        let bog = parse_bog(input).unwrap();
        match &bog.annotations[0] {
            Annotation::Repo(r) => {
                assert_eq!(r.name, "my-project");
                assert_eq!(r.version, "0.1.0");
            }
            _ => panic!("expected Repo annotation"),
        }
    }

    #[test]
    fn test_parse_subsystem() {
        let input = r#"
#[subsystem(authentication) {
  owner = "auth-agent",
  files = ["src/auth/*.rs", "src/auth.rs"],
  status = green,
  description = "JWT-based user authentication"
}]
"#;
        let bog = parse_bog(input).unwrap();
        match &bog.annotations[0] {
            Annotation::Subsystem(s) => {
                assert_eq!(s.name, "authentication");
                assert_eq!(s.owner, "auth-agent");
                assert_eq!(s.files.len(), 2);
                assert_eq!(s.status, Status::Green);
            }
            _ => panic!("expected Subsystem annotation"),
        }
    }

    #[test]
    fn test_parse_policies() {
        let input = r#"
#[policies {
  require_contracts = true,
  require_owner = true,
  health_thresholds = {
    red_max_days = 7,
    stale_after_days = 30
  }
}]
"#;
        let bog = parse_bog(input).unwrap();
        match &bog.annotations[0] {
            Annotation::Policies(p) => {
                assert_eq!(p.fields["require_contracts"], Value::Bool(true));
                assert_eq!(p.fields["require_owner"], Value::Bool(true));
                assert!(matches!(p.fields.get("health_thresholds"), Some(Value::Block(_))));
            }
            _ => panic!("expected Policies annotation"),
        }
    }

    #[test]
    fn test_parse_change_requests() {
        let input = r#"
#[change_requests {
  #[request(
    id = "cr-001",
    from = "api-agent",
    target = fn(login),
    type = modify_contract,
    status = pending,
    priority = medium,
    created = "2026-02-18",
    description = "Need login to accept OAuth tokens too"
  )]
}]
"#;
        let bog = parse_bog(input).unwrap();
        assert_eq!(bog.annotations.len(), 1);
        match &bog.annotations[0] {
            Annotation::ChangeRequests(reqs) => {
                assert_eq!(reqs.len(), 1);
                assert_eq!(reqs[0].id, "cr-001");
                assert_eq!(reqs[0].from, "api-agent");
                assert_eq!(reqs[0].change_type, "modify_contract");
                assert_eq!(reqs[0].status, "pending");
                assert!(matches!(&reqs[0].target, Value::FnRef(name) if name == "login"));
            }
            _ => panic!("expected ChangeRequests annotation"),
        }
    }

    #[test]
    fn test_parse_with_comments() {
        let input = r#"
// This is a comment
#[health(
  test_coverage = green,
  staleness = green,
  complexity = green,
  // custom dimension
  security_audit = green
)]
"#;
        let bog = parse_bog(input).unwrap();
        assert_eq!(bog.annotations.len(), 1);
        match &bog.annotations[0] {
            Annotation::Health(h) => {
                assert_eq!(h.dimensions.len(), 4);
                assert_eq!(h.dimensions["security_audit"], Status::Green);
            }
            _ => panic!("expected Health annotation"),
        }
    }

    #[test]
    fn test_parse_repo_bog() {
        let input = r#"
#[repo(
  name = "my-project",
  version = "0.1.0",
  updated = "2026-02-18"
)]

#[description {
  A web API for user management with authentication and authorization.
}]

#[subsystem(authentication) {
  owner = "auth-agent",
  files = ["src/auth/*.rs", "src/auth.rs"],
  status = green,
  description = "JWT-based user authentication"
}]

#[subsystem(database) {
  owner = "db-agent",
  files = ["src/db/*.rs", "src/db.rs"],
  status = yellow,
  description = "PostgreSQL data layer"
}]

#[policies {
  require_contracts = true,
  require_owner = true,
  health_thresholds = {
    red_max_days = 7,
    stale_after_days = 30
  }
}]
"#;
        let bog = parse_bog(input).unwrap();
        assert_eq!(bog.annotations.len(), 5);

        // Check repo
        assert!(matches!(&bog.annotations[0], Annotation::Repo(_)));
        // Check description
        assert!(matches!(&bog.annotations[1], Annotation::Description(_)));
        // Check subsystems
        assert!(matches!(&bog.annotations[2], Annotation::Subsystem(s) if s.name == "authentication"));
        assert!(matches!(&bog.annotations[3], Annotation::Subsystem(s) if s.name == "database"));
        // Check policies
        assert!(matches!(&bog.annotations[4], Annotation::Policies(_)));
    }

    #[test]
    fn test_parse_skimsystem() {
        let input = r#"
#[skimsystem(test-concision) {
  owner = "quality-agent",
  targets = [core, analysis, cli],
  status = green,
  principles = [
    "Each test should verify exactly one behavior",
    "Test names should describe expected behavior"
  ],
  description = "Ensures test suites remain focused and readable"
}]
"#;
        let bog = parse_bog(input).unwrap();
        assert_eq!(bog.annotations.len(), 1);
        match &bog.annotations[0] {
            Annotation::Skimsystem(s) => {
                assert_eq!(s.name, "test-concision");
                assert_eq!(s.owner, "quality-agent");
                assert_eq!(s.targets, SkimTargets::Named(vec![
                    "core".to_string(),
                    "analysis".to_string(),
                    "cli".to_string(),
                ]));
                assert_eq!(s.status, Status::Green);
                assert_eq!(s.principles.len(), 2);
                assert!(s.principles[0].contains("one behavior"));
            }
            _ => panic!("expected Skimsystem annotation"),
        }
    }

    #[test]
    fn test_parse_skimsystem_targets_all() {
        let input = r#"
#[skimsystem(observability) {
  owner = "ops-agent",
  targets = all,
  status = green,
  description = "Ensures structured logging everywhere"
}]
"#;
        let bog = parse_bog(input).unwrap();
        match &bog.annotations[0] {
            Annotation::Skimsystem(s) => {
                assert_eq!(s.targets, SkimTargets::All);
                assert!(s.principles.is_empty());
            }
            _ => panic!("expected Skimsystem annotation"),
        }
    }

    #[test]
    fn test_parse_skimsystem_with_integrations() {
        let input = r#"
#[skimsystem(code-quality) {
  owner = "quality-agent",
  targets = all,
  status = green,
  integrations = {
    clippy = {
      command = "cargo clippy --message-format=json -- -W clippy::pedantic",
      format = cargo_diagnostic
    }
  },
  principles = ["No pedantic clippy warnings"],
  description = "Runs pedantic clippy"
}]
"#;
        let bog = parse_bog(input).unwrap();
        match &bog.annotations[0] {
            Annotation::Skimsystem(s) => {
                assert_eq!(s.name, "code-quality");
                assert_eq!(s.integrations.len(), 1);
                assert_eq!(s.integrations[0].name, "clippy");
                assert!(s.integrations[0].command.contains("clippy"));
                assert_eq!(s.integrations[0].format, IntegrationFormat::CargoDiagnostic);
            }
            _ => panic!("expected Skimsystem annotation"),
        }
    }

    #[test]
    fn test_parse_skim_observation() {
        let input = r#"
#[file(
  owner = "auth-agent",
  subsystem = "authentication",
  updated = "2026-02-18",
  status = green
)]

#[skim(test-concision) {
  status = yellow,
  notes = "test_config_loading verifies multiple behaviors"
}]
"#;
        let bog = parse_bog(input).unwrap();
        assert_eq!(bog.annotations.len(), 2);
        match &bog.annotations[1] {
            Annotation::Skim(obs) => {
                assert_eq!(obs.skimsystem, "test-concision");
                assert_eq!(obs.status, Status::Yellow);
                assert!(obs.notes.as_ref().unwrap().contains("multiple behaviors"));
                assert!(obs.target.is_none());
            }
            _ => panic!("expected Skim annotation"),
        }
    }

    #[test]
    fn test_parse_skim_with_fn_target() {
        let input = r#"
#[skim(test-concision) {
  status = red,
  target = fn(login),
  notes = "Function too complex"
}]
"#;
        let bog = parse_bog(input).unwrap();
        match &bog.annotations[0] {
            Annotation::Skim(obs) => {
                assert_eq!(obs.target, Some(SkimTarget::Fn("login".to_string())));
                assert_eq!(obs.status, Status::Red);
            }
            _ => panic!("expected Skim annotation"),
        }
    }

    #[test]
    fn test_parse_pickled_decision() {
        let input = r#"
#[pickled(agent = "core-agent", updated = "2026-02-24") {
  id = "pickle-001",
  kind = decision,
  tags = [architecture, reliability],
  content = "Keep single PEG grammar rather than splitting per annotation type. Splitting looked cleaner but made error recovery much harder."
}]
"#;
        let bog = parse_bog(input).unwrap();
        assert_eq!(bog.annotations.len(), 1);
        match &bog.annotations[0] {
            Annotation::Pickled(p) => {
                assert_eq!(p.id, "pickle-001");
                assert_eq!(p.agent, "core-agent");
                assert_eq!(p.updated, "2026-02-24");
                assert_eq!(p.kind, PickledKind::Decision);
                assert_eq!(p.supersedes, None);
                assert_eq!(p.tags, vec![PickledTag::Architecture, PickledTag::Reliability]);
                assert!(p.content.contains("single PEG grammar"));
            }
            _ => panic!("expected Pickled annotation"),
        }
    }

    #[test]
    fn test_parse_pickled_reversal() {
        let input = r#"
#[pickled(agent = "core-agent", updated = "2026-02-26") {
  id = "pickle-003",
  kind = reversal,
  supersedes = "pickle-002",
  tags = [architecture],
  content = "Reverting nested grammar experiment. Error messages became useless."
}]
"#;
        let bog = parse_bog(input).unwrap();
        match &bog.annotations[0] {
            Annotation::Pickled(p) => {
                assert_eq!(p.kind, PickledKind::Reversal);
                assert_eq!(p.supersedes, Some("pickle-002".to_string()));
            }
            _ => panic!("expected Pickled annotation"),
        }
    }

    #[test]
    fn test_parse_pickled_context() {
        let input = r#"
#[pickled(agent = "analysis-agent", updated = "2026-02-25") {
  id = "pickle-010",
  kind = context,
  tags = [domain],
  content = "Tree-sitter queries for Rust impl blocks need special handling — methods inside impl are function_item nodes nested under impl_item."
}]
"#;
        let bog = parse_bog(input).unwrap();
        match &bog.annotations[0] {
            Annotation::Pickled(p) => {
                assert_eq!(p.kind, PickledKind::Context);
                assert_eq!(p.tags, vec![PickledTag::Domain]);
                assert!(p.content.contains("impl blocks"));
            }
            _ => panic!("expected Pickled annotation"),
        }
    }

    #[test]
    fn test_parse_multiple_pickled() {
        let input = r#"
#[pickled(agent = "core-agent", updated = "2026-02-20") {
  id = "pickle-001",
  kind = observation,
  tags = [architecture],
  content = "parser.pest uses a single annotation rule for all types — dispatch happens in Rust."
}]

#[pickled(agent = "core-agent", updated = "2026-02-24") {
  id = "pickle-002",
  kind = decision,
  tags = [architecture, debt],
  content = "Contract parsing uses nested blocks. Accepted complexity tradeoff for expressiveness."
}]
"#;
        let bog = parse_bog(input).unwrap();
        assert_eq!(bog.annotations.len(), 2);
        match &bog.annotations[0] {
            Annotation::Pickled(p) => {
                assert_eq!(p.id, "pickle-001");
                assert_eq!(p.kind, PickledKind::Observation);
            }
            _ => panic!("expected Pickled annotation"),
        }
        match &bog.annotations[1] {
            Annotation::Pickled(p) => {
                assert_eq!(p.id, "pickle-002");
                assert_eq!(p.kind, PickledKind::Decision);
                assert_eq!(p.tags, vec![PickledTag::Architecture, PickledTag::Debt]);
            }
            _ => panic!("expected Pickled annotation"),
        }
    }

    #[test]
    fn test_parse_full_file_bog() {
        let input = r#"
#[file(
  owner = "auth-agent",
  subsystem = "authentication",
  updated = "2026-02-18",
  status = green
)]

#[description {
  Handles user authentication via JWT.
}]

#[health(
  test_coverage = green,
  staleness = green
)]

#[fn(login) {
  status = green,
  deps = [db::get_user],
  description = "Authenticates user"
}]
"#;
        let bog = parse_bog(input).unwrap();
        assert_eq!(bog.annotations.len(), 4);
    }
}
