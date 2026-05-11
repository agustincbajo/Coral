//! Repo symbol map — lightweight symbol extraction from source files.
//!
//! Uses regex patterns (no tree-sitter dependency) to extract top-level
//! declarations from Rust, TypeScript, Python, and Go source files.
//! Produces a flat list of `Symbol` entries that `coral ingest` can
//! cross-reference against wiki page slugs and sources.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// A source code symbol declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub file: PathBuf,
    pub line: usize,
    /// The module/namespace path (e.g., "crate::auth::handler")
    pub module_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Struct,
    Trait,
    Enum,
    Type,
    Const,
    Module,
    Class,
    Interface,
    Method,
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Function => "function",
            Self::Struct => "struct",
            Self::Trait => "trait",
            Self::Enum => "enum",
            Self::Type => "type",
            Self::Const => "const",
            Self::Module => "module",
            Self::Class => "class",
            Self::Interface => "interface",
            Self::Method => "method",
        };
        f.write_str(s)
    }
}

/// Directories to always skip during recursive extraction.
const SKIP_DIRS: &[&str] = &["node_modules", "target", ".git", "vendor"];

// ─── Extraction ─────────────────────────────────────────────────────────────

/// Extract symbols from a single file. Language is detected by extension.
pub fn extract_from_file(path: &Path) -> Vec<Symbol> {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e.to_lowercase(),
        None => return vec![],
    };

    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    match ext.as_str() {
        "rs" => extract_rust(&content, path),
        "ts" | "tsx" => extract_typescript(&content, path),
        "py" => extract_python(&content, path),
        "go" => extract_go(&content, path),
        _ => vec![],
    }
}

/// Walk `root` recursively, extract symbols from files matching `extensions`.
/// Skips `node_modules/`, `target/`, `.git/`, `vendor/`.
pub fn extract_from_dir(root: &Path, extensions: &[&str]) -> Vec<Symbol> {
    let mut symbols = Vec::new();

    for entry in WalkDir::new(root).follow_links(false).into_iter() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Skip excluded directories.
        if entry.file_type().is_dir() {
            let name = entry.file_name().to_string_lossy();
            if SKIP_DIRS.contains(&name.as_ref()) {
                // WalkDir doesn't support skip_current_dir on owned iterator
                // but we filter below.
                continue;
            }
        }

        // Check if any ancestor is a skip dir.
        let path = entry.path();
        if path.ancestors().any(|a| {
            a.file_name()
                .and_then(|n| n.to_str())
                .map(|n| SKIP_DIRS.contains(&n))
                .unwrap_or(false)
        }) {
            continue;
        }

        if !entry.file_type().is_file() {
            continue;
        }

        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e,
            None => continue,
        };

        if extensions.contains(&ext) {
            symbols.extend(extract_from_file(path));
        }
    }

    symbols
}

// ─── Indexing ───────────────────────────────────────────────────────────────

/// Build a lookup index from symbol name -> list of symbols with that name.
pub fn build_symbol_index(symbols: &[Symbol]) -> HashMap<String, Vec<&Symbol>> {
    let mut index: HashMap<String, Vec<&Symbol>> = HashMap::new();
    for sym in symbols {
        index
            .entry(sym.name.to_lowercase())
            .or_default()
            .push(sym);
    }
    index
}

/// Find symbols whose name matches the slug (case-insensitive, with
/// underscore/hyphen normalization). Used to auto-link wiki pages to source.
pub fn find_symbols_for_slug<'a>(symbols: &'a [Symbol], slug: &str) -> Vec<&'a Symbol> {
    let normalized = normalize_slug(slug);
    symbols
        .iter()
        .filter(|s| normalize_slug(&s.name) == normalized)
        .collect()
}

/// Normalize a slug/name for comparison: insert separators at camelCase
/// boundaries, lowercase, and collapse all separator variants (hyphens,
/// underscores) into a single canonical separator.
fn normalize_slug(s: &str) -> String {
    // Step 1: Insert underscores at camelCase boundaries.
    let mut expanded = String::with_capacity(s.len() + 4);
    let chars: Vec<char> = s.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if i > 0
            && ch.is_uppercase()
            && !chars[i - 1].is_uppercase()
            && chars[i - 1] != '_'
            && chars[i - 1] != '-'
        {
            expanded.push('_');
        }
        expanded.push(ch);
    }
    // Step 2: Lowercase, replace hyphens with underscores, collapse runs.
    let lower = expanded.to_lowercase().replace('-', "_");
    // Collapse consecutive underscores into one.
    let mut result = String::with_capacity(lower.len());
    let mut prev_underscore = false;
    for ch in lower.chars() {
        if ch == '_' {
            if !prev_underscore {
                result.push('_');
            }
            prev_underscore = true;
        } else {
            result.push(ch);
            prev_underscore = false;
        }
    }
    result
}

// ─── Rendering ──────────────────────────────────────────────────────────────

/// Render a markdown summary table of extracted symbols.
pub fn render_markdown(symbols: &[Symbol]) -> String {
    if symbols.is_empty() {
        return String::from("_No symbols found._\n");
    }

    let mut out = String::new();
    out.push_str("| Symbol | Kind | File | Line |\n");
    out.push_str("|--------|------|------|------|\n");
    for sym in symbols {
        out.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            sym.name,
            sym.kind,
            sym.file.display(),
            sym.line,
        ));
    }
    out
}

/// Render symbols as a JSON array.
pub fn render_json(symbols: &[Symbol]) -> serde_json::Value {
    serde_json::to_value(symbols).unwrap_or(serde_json::Value::Array(vec![]))
}

// ─── Language-specific extractors ───────────────────────────────────────────

fn extract_rust(content: &str, path: &Path) -> Vec<Symbol> {
    let re = Regex::new(
        r"(?m)^[ \t]*pub\s+(?:(?:async|unsafe|const)\s+)*(?:(fn|struct|trait|enum|type|const|mod))\s+([A-Za-z_][A-Za-z0-9_]*)",
    )
    .expect("valid regex");

    let mut symbols = Vec::new();
    for (line_idx, line) in content.lines().enumerate() {
        if let Some(caps) = re.captures(line) {
            let kind_str = caps.get(1).unwrap().as_str();
            let name = caps.get(2).unwrap().as_str().to_string();
            let kind = match kind_str {
                "fn" => SymbolKind::Function,
                "struct" => SymbolKind::Struct,
                "trait" => SymbolKind::Trait,
                "enum" => SymbolKind::Enum,
                "type" => SymbolKind::Type,
                "const" => SymbolKind::Const,
                "mod" => SymbolKind::Module,
                _ => continue,
            };
            symbols.push(Symbol {
                name,
                kind,
                file: path.to_path_buf(),
                line: line_idx + 1,
                module_path: None,
            });
        }
    }
    symbols
}

fn extract_typescript(content: &str, path: &Path) -> Vec<Symbol> {
    let re = Regex::new(
        r"(?m)^[ \t]*export\s+(?:default\s+)?(?:declare\s+)?(?:abstract\s+)?(function|class|interface|type|const|enum)\s+([A-Za-z_$][A-Za-z0-9_$]*)",
    )
    .expect("valid regex");

    let mut symbols = Vec::new();
    for (line_idx, line) in content.lines().enumerate() {
        if let Some(caps) = re.captures(line) {
            let kind_str = caps.get(1).unwrap().as_str();
            let name = caps.get(2).unwrap().as_str().to_string();
            let kind = match kind_str {
                "function" => SymbolKind::Function,
                "class" => SymbolKind::Class,
                "interface" => SymbolKind::Interface,
                "type" => SymbolKind::Type,
                "const" => SymbolKind::Const,
                "enum" => SymbolKind::Enum,
                _ => continue,
            };
            symbols.push(Symbol {
                name,
                kind,
                file: path.to_path_buf(),
                line: line_idx + 1,
                module_path: None,
            });
        }
    }
    symbols
}

fn extract_python(content: &str, path: &Path) -> Vec<Symbol> {
    let re_def = Regex::new(r"(?m)^(def|class)\s+([A-Za-z_][A-Za-z0-9_]*)").expect("valid regex");
    let re_const =
        Regex::new(r"(?m)^([A-Z][A-Z0-9_]*)\s*=").expect("valid regex");

    let mut symbols = Vec::new();
    for (line_idx, line) in content.lines().enumerate() {
        if let Some(caps) = re_def.captures(line) {
            let kind_str = caps.get(1).unwrap().as_str();
            let name = caps.get(2).unwrap().as_str().to_string();
            let kind = match kind_str {
                "def" => SymbolKind::Function,
                "class" => SymbolKind::Class,
                _ => continue,
            };
            symbols.push(Symbol {
                name,
                kind,
                file: path.to_path_buf(),
                line: line_idx + 1,
                module_path: None,
            });
        } else if let Some(caps) = re_const.captures(line) {
            let name = caps.get(1).unwrap().as_str().to_string();
            symbols.push(Symbol {
                name,
                kind: SymbolKind::Const,
                file: path.to_path_buf(),
                line: line_idx + 1,
                module_path: None,
            });
        }
    }
    symbols
}

fn extract_go(content: &str, path: &Path) -> Vec<Symbol> {
    let re_func =
        Regex::new(r"(?m)^func\s+(?:\([^)]*\)\s+)?([A-Za-z_][A-Za-z0-9_]*)").expect("valid regex");
    let re_type_struct =
        Regex::new(r"(?m)^type\s+([A-Za-z_][A-Za-z0-9_]*)\s+struct\b").expect("valid regex");
    let re_type_interface =
        Regex::new(r"(?m)^type\s+([A-Za-z_][A-Za-z0-9_]*)\s+interface\b").expect("valid regex");

    let mut symbols = Vec::new();
    for (line_idx, line) in content.lines().enumerate() {
        if let Some(caps) = re_type_struct.captures(line) {
            let name = caps.get(1).unwrap().as_str().to_string();
            symbols.push(Symbol {
                name,
                kind: SymbolKind::Struct,
                file: path.to_path_buf(),
                line: line_idx + 1,
                module_path: None,
            });
        } else if let Some(caps) = re_type_interface.captures(line) {
            let name = caps.get(1).unwrap().as_str().to_string();
            symbols.push(Symbol {
                name,
                kind: SymbolKind::Interface,
                file: path.to_path_buf(),
                line: line_idx + 1,
                module_path: None,
            });
        } else if let Some(caps) = re_func.captures(line) {
            let name = caps.get(1).unwrap().as_str().to_string();
            // Methods have a receiver — detect via the `(receiver)` prefix.
            let kind = if line.starts_with("func (") {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            };
            symbols.push(Symbol {
                name,
                kind,
                file: path.to_path_buf(),
                line: line_idx + 1,
                module_path: None,
            });
        }
    }
    symbols
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_extract_rust() {
        let src = r#"
use std::io;

pub fn handle_request(req: Request) -> Response {
    todo!()
}

pub async fn fetch_data() -> Result<()> {
    Ok(())
}

pub struct Config {
    name: String,
}

pub trait Handler {
    fn handle(&self);
}

pub enum Status {
    Active,
    Inactive,
}

pub type Id = u64;

pub const MAX_RETRIES: u32 = 3;

pub mod auth;

fn private_fn() {}
"#;
        let path = Path::new("src/lib.rs");
        let symbols = extract_rust(src, path);

        assert_eq!(symbols.len(), 8);
        assert_eq!(symbols[0].name, "handle_request");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[0].line, 4);

        assert_eq!(symbols[1].name, "fetch_data");
        assert_eq!(symbols[1].kind, SymbolKind::Function);

        assert_eq!(symbols[2].name, "Config");
        assert_eq!(symbols[2].kind, SymbolKind::Struct);

        assert_eq!(symbols[3].name, "Handler");
        assert_eq!(symbols[3].kind, SymbolKind::Trait);

        assert_eq!(symbols[4].name, "Status");
        assert_eq!(symbols[4].kind, SymbolKind::Enum);

        assert_eq!(symbols[5].name, "Id");
        assert_eq!(symbols[5].kind, SymbolKind::Type);

        assert_eq!(symbols[6].name, "MAX_RETRIES");
        assert_eq!(symbols[6].kind, SymbolKind::Const);

        assert_eq!(symbols[7].name, "auth");
        assert_eq!(symbols[7].kind, SymbolKind::Module);
    }

    #[test]
    fn test_extract_typescript() {
        let src = r#"
import { foo } from './bar';

export function handleRequest(req: Request): Response {
    return new Response();
}

export class AuthService {
    constructor() {}
}

export interface Config {
    name: string;
}

export type UserId = string;

export const MAX_RETRIES = 3;

export enum Status {
    Active,
    Inactive,
}

export default function main() {}

function privateHelper() {}
"#;
        let path = Path::new("src/index.ts");
        let symbols = extract_typescript(src, path);

        assert_eq!(symbols.len(), 7);
        assert_eq!(symbols[0].name, "handleRequest");
        assert_eq!(symbols[0].kind, SymbolKind::Function);

        assert_eq!(symbols[1].name, "AuthService");
        assert_eq!(symbols[1].kind, SymbolKind::Class);

        assert_eq!(symbols[2].name, "Config");
        assert_eq!(symbols[2].kind, SymbolKind::Interface);

        assert_eq!(symbols[3].name, "UserId");
        assert_eq!(symbols[3].kind, SymbolKind::Type);

        assert_eq!(symbols[4].name, "MAX_RETRIES");
        assert_eq!(symbols[4].kind, SymbolKind::Const);

        assert_eq!(symbols[5].name, "Status");
        assert_eq!(symbols[5].kind, SymbolKind::Enum);

        assert_eq!(symbols[6].name, "main");
        assert_eq!(symbols[6].kind, SymbolKind::Function);
    }

    #[test]
    fn test_extract_python() {
        let src = r#"
import os

MAX_RETRIES = 3
DEFAULT_TIMEOUT = 30

def handle_request(req):
    pass

class AuthService:
    def __init__(self):
        pass

    def authenticate(self):
        pass

def _private():
    pass
"#;
        let path = Path::new("app/main.py");
        let symbols = extract_python(src, path);

        // Top-level declarations only: 2 consts + 2 functions + 1 class = 5.
        // Indented methods (__init__, authenticate) are not captured because
        // the regex anchors `def`/`class` at the start of line (`^`).
        assert_eq!(symbols.len(), 5);
        assert_eq!(symbols[0].name, "MAX_RETRIES");
        assert_eq!(symbols[0].kind, SymbolKind::Const);

        assert_eq!(symbols[1].name, "DEFAULT_TIMEOUT");
        assert_eq!(symbols[1].kind, SymbolKind::Const);

        assert_eq!(symbols[2].name, "handle_request");
        assert_eq!(symbols[2].kind, SymbolKind::Function);

        assert_eq!(symbols[3].name, "AuthService");
        assert_eq!(symbols[3].kind, SymbolKind::Class);

        assert_eq!(symbols[4].name, "_private");
        assert_eq!(symbols[4].kind, SymbolKind::Function);
    }

    #[test]
    fn test_extract_go() {
        let src = r#"
package main

import "fmt"

func main() {
    fmt.Println("hello")
}

func HandleRequest(w http.ResponseWriter, r *http.Request) {
}

type Config struct {
    Name string
}

type Handler interface {
    Handle()
}

func (c *Config) Validate() error {
    return nil
}
"#;
        let path = Path::new("main.go");
        let symbols = extract_go(src, path);

        assert_eq!(symbols.len(), 5);

        // Symbols appear in file order.
        assert_eq!(symbols[0].name, "main");
        assert_eq!(symbols[0].kind, SymbolKind::Function);

        assert_eq!(symbols[1].name, "HandleRequest");
        assert_eq!(symbols[1].kind, SymbolKind::Function);

        assert_eq!(symbols[2].name, "Config");
        assert_eq!(symbols[2].kind, SymbolKind::Struct);

        assert_eq!(symbols[3].name, "Handler");
        assert_eq!(symbols[3].kind, SymbolKind::Interface);

        assert_eq!(symbols[4].name, "Validate");
        assert_eq!(symbols[4].kind, SymbolKind::Method);
    }

    #[test]
    fn test_find_symbols_for_slug() {
        let symbols = vec![
            Symbol {
                name: "handle_request".to_string(),
                kind: SymbolKind::Function,
                file: PathBuf::from("src/lib.rs"),
                line: 10,
                module_path: None,
            },
            Symbol {
                name: "HandleRequest".to_string(),
                kind: SymbolKind::Function,
                file: PathBuf::from("main.go"),
                line: 5,
                module_path: None,
            },
            Symbol {
                name: "AuthService".to_string(),
                kind: SymbolKind::Class,
                file: PathBuf::from("app.ts"),
                line: 1,
                module_path: None,
            },
        ];

        // Hyphen slug matches underscore name (case-insensitive).
        let matches = find_symbols_for_slug(&symbols, "handle-request");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].name, "handle_request");
        assert_eq!(matches[1].name, "HandleRequest");

        // Underscore slug matches too.
        let matches = find_symbols_for_slug(&symbols, "Handle_Request");
        assert_eq!(matches.len(), 2);

        // Exact match (case-insensitive).
        let matches = find_symbols_for_slug(&symbols, "auth-service");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "AuthService");

        // No match.
        let matches = find_symbols_for_slug(&symbols, "nonexistent");
        assert!(matches.is_empty());
    }

    #[test]
    fn test_extract_from_dir_skips_excluded() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Create some source files.
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "pub fn hello() {}\n",
        )
        .unwrap();

        // Create files in excluded directories.
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::write(
            root.join("target/debug/build.rs"),
            "pub fn build_artifact() {}\n",
        )
        .unwrap();

        fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        fs::write(
            root.join("node_modules/pkg/index.ts"),
            "export function dep() {}\n",
        )
        .unwrap();

        fs::create_dir_all(root.join(".git/objects")).unwrap();
        fs::write(
            root.join(".git/objects/pack.rs"),
            "pub fn git_internal() {}\n",
        )
        .unwrap();

        fs::create_dir_all(root.join("vendor/lib")).unwrap();
        fs::write(
            root.join("vendor/lib/dep.go"),
            "func vendored() {}\n",
        )
        .unwrap();

        let symbols = extract_from_dir(root, &["rs", "ts", "go"]);

        // Only src/lib.rs should be picked up.
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
    }

    #[test]
    fn test_build_symbol_index() {
        let symbols = vec![
            Symbol {
                name: "Config".to_string(),
                kind: SymbolKind::Struct,
                file: PathBuf::from("a.rs"),
                line: 1,
                module_path: None,
            },
            Symbol {
                name: "config".to_string(),
                kind: SymbolKind::Function,
                file: PathBuf::from("b.rs"),
                line: 5,
                module_path: None,
            },
        ];

        let index = build_symbol_index(&symbols);
        // Both "Config" and "config" normalize to "config".
        assert_eq!(index.get("config").unwrap().len(), 2);
    }

    #[test]
    fn test_render_markdown() {
        let symbols = vec![Symbol {
            name: "hello".to_string(),
            kind: SymbolKind::Function,
            file: PathBuf::from("src/lib.rs"),
            line: 3,
            module_path: None,
        }];
        let md = render_markdown(&symbols);
        assert!(md.contains("| `hello` | function | src/lib.rs | 3 |"));
    }

    #[test]
    fn test_render_json() {
        let symbols = vec![Symbol {
            name: "Foo".to_string(),
            kind: SymbolKind::Struct,
            file: PathBuf::from("lib.rs"),
            line: 1,
            module_path: None,
        }];
        let json = render_json(&symbols);
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "Foo");
        assert_eq!(arr[0]["kind"], "struct");
    }
}
