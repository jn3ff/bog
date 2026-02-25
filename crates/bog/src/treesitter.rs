use std::collections::BTreeSet;

use tree_sitter::Parser;

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub params: Vec<(String, String)>,
    pub return_type: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub calls: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Method,
}

#[derive(Debug, thiserror::Error)]
pub enum TreeSitterError {
    #[error("Failed to initialize parser: {0}")]
    Init(String),

    #[error("Failed to parse source file")]
    ParseFailed,
}

pub fn extract_symbols(source: &str) -> Result<Vec<Symbol>, TreeSitterError> {
    let mut parser = Parser::new();
    let language = tree_sitter_rust::LANGUAGE;
    parser
        .set_language(&language.into())
        .map_err(|e| TreeSitterError::Init(e.to_string()))?;

    let tree = parser
        .parse(source, None)
        .ok_or(TreeSitterError::ParseFailed)?;

    let mut symbols = Vec::new();
    collect_symbols(tree.root_node(), source.as_bytes(), &mut symbols);
    Ok(symbols)
}

fn collect_symbols(node: tree_sitter::Node, source: &[u8], symbols: &mut Vec<Symbol>) {
    match node.kind() {
        "function_item" => {
            if let Some(sym) = extract_function(node, source) {
                symbols.push(sym);
            }
        }
        "impl_item" => {
            for i in 0..node.child_count() {
                let child = node.child(i).unwrap();
                if child.kind() == "declaration_list" {
                    for j in 0..child.child_count() {
                        let item = child.child(j).unwrap();
                        if item.kind() == "function_item" {
                            if let Some(mut sym) = extract_function(item, source) {
                                sym.kind = SymbolKind::Method;
                                symbols.push(sym);
                            }
                        }
                    }
                }
            }
        }
        _ => {
            for i in 0..node.child_count() {
                collect_symbols(node.child(i).unwrap(), source, symbols);
            }
        }
    }
}

fn extract_function(node: tree_sitter::Node, source: &[u8]) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source).ok()?.to_string();

    let mut params = Vec::new();
    if let Some(param_list) = node.child_by_field_name("parameters") {
        for i in 0..param_list.child_count() {
            let param = param_list.child(i).unwrap();
            if param.kind() == "parameter" {
                let param_name = param
                    .child_by_field_name("pattern")
                    .map(|n| n.utf8_text(source).unwrap_or("_").to_string())
                    .unwrap_or_else(|| "_".to_string());
                let param_type = param
                    .child_by_field_name("type")
                    .map(|n| n.utf8_text(source).unwrap_or("?").to_string())
                    .unwrap_or_else(|| "?".to_string());
                params.push((param_name, param_type));
            }
        }
    }

    let return_type = node
        .child_by_field_name("return_type")
        .map(|n| n.utf8_text(source).unwrap_or("").to_string());

    // Extract function calls from the body
    let mut calls = BTreeSet::new();
    if let Some(body) = node.child_by_field_name("body") {
        extract_calls(body, source, &mut calls);
    }
    // Remove self-references
    calls.remove(&name);

    Some(Symbol {
        name,
        kind: SymbolKind::Function,
        params,
        return_type,
        start_line: name_node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        calls: calls.into_iter().collect(),
    })
}

fn extract_calls(node: tree_sitter::Node, source: &[u8], calls: &mut BTreeSet<String>) {
    if node.kind() == "call_expression" {
        if let Some(func) = node.child_by_field_name("function") {
            match func.kind() {
                "identifier" => {
                    if let Ok(name) = func.utf8_text(source) {
                        // Skip common non-function identifiers
                        if !is_builtin(name) {
                            calls.insert(name.to_string());
                        }
                    }
                }
                "scoped_identifier" => {
                    if let Ok(path) = func.utf8_text(source) {
                        // Skip Self::, self::, and stdlib type constructors
                        let root_segment = path.split("::").next().unwrap_or("");
                        if !path.starts_with("Self::")
                            && !path.starts_with("self::")
                            && !is_builtin(root_segment)
                        {
                            calls.insert(path.to_string());
                        }
                    }
                }
                // Skip field_expression (method calls on objects like obj.method())
                _ => {}
            }
        }
    }

    for i in 0..node.child_count() {
        extract_calls(node.child(i).unwrap(), source, calls);
    }
}

fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "Some" | "None" | "Ok" | "Err" | "Box" | "Vec" | "String" | "format" | "println"
            | "eprintln" | "print" | "eprint" | "write" | "writeln" | "panic" | "todo"
            | "unimplemented" | "unreachable" | "assert" | "assert_eq" | "assert_ne"
            | "debug_assert" | "debug_assert_eq" | "debug_assert_ne" | "cfg" | "include"
            | "include_str" | "include_bytes" | "env" | "option_env" | "concat" | "line"
            | "column" | "file" | "stringify" | "module_path"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_functions() {
        let source = r#"
fn login(username: &str, password: &str) -> Result<Token, AuthError> {
    todo!()
}

fn logout(token: Token) {
    todo!()
}
"#;
        let symbols = extract_symbols(source).unwrap();
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "login");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[1].name, "logout");
    }

    #[test]
    fn test_extract_methods() {
        let source = r#"
struct Auth;

impl Auth {
    fn new() -> Self {
        Auth
    }

    fn verify(&self, token: &str) -> bool {
        true
    }
}
"#;
        let symbols = extract_symbols(source).unwrap();
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "new");
        assert_eq!(symbols[0].kind, SymbolKind::Method);
        assert_eq!(symbols[1].name, "verify");
        assert_eq!(symbols[1].kind, SymbolKind::Method);
    }

    #[test]
    fn test_extract_calls() {
        let source = r#"
fn process(input: &str) -> Result<Output, Error> {
    let parsed = parse_input(input);
    let validated = validator::check(parsed);
    let result = validated.transform();
    db::store(result)
}
"#;
        let symbols = extract_symbols(source).unwrap();
        assert_eq!(symbols.len(), 1);
        let calls = &symbols[0].calls;
        assert!(calls.contains(&"parse_input".to_string()));
        assert!(calls.contains(&"validator::check".to_string()));
        assert!(calls.contains(&"db::store".to_string()));
        // method call .transform() should NOT be included
        assert!(!calls.iter().any(|c| c.contains("transform")));
    }

    #[test]
    fn test_skips_self_and_builtins() {
        let source = r#"
fn example() {
    let v = Vec::new();
    let s = Some(42);
    println!("hello");
    Self::helper();
    real_function();
}
"#;
        let symbols = extract_symbols(source).unwrap();
        let calls = &symbols[0].calls;
        assert!(calls.contains(&"real_function".to_string()));
        assert!(!calls.iter().any(|c| c.contains("Vec")));
        assert!(!calls.iter().any(|c| c.contains("Some")));
        assert!(!calls.iter().any(|c| c.contains("Self")));
    }
}
