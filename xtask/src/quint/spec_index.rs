//! Lexical index of a Quint spec file: its modules, whether each declares a
//! `step` relation, the named `run`s each declares, and the files it imports.
//!
//! Used by `cargo xtask quint` (every stateful module carries named runs, and
//! every named run is listed in `checks.toml`) and by `cargo xtask
//! conformance-lint` (a `#[quint_test]` names a run the spec defines). Both
//! need only declarations at a module's top level, so a lexer that skips
//! comments and strings and tracks braces is enough; neither gate needs to
//! start Quint. The index fails closed: an unterminated comment or string,
//! or unbalanced braces, is an error rather than a partial index.

use std::fmt;

/// One `run` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QntRun {
    /// The run's name.
    pub name: String,
    /// 1-based line of the `run` keyword.
    pub line: usize,
}

/// One `module` of a spec file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QntModule {
    /// The module's name.
    pub name: String,
    /// 1-based line of the `module` keyword.
    pub line: usize,
    /// The module declares `action step` itself (an `import … .*` of a
    /// module that does is not a declaration).
    pub step: bool,
    /// The named runs the module declares, in source order.
    pub runs: Vec<QntRun>,
}

/// The index of one spec file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QntIndex {
    /// Modules in source order.
    pub modules: Vec<QntModule>,
    /// Paths named by `from "<path>"` in any import, relative to the file.
    pub imports: Vec<String>,
}

impl QntIndex {
    /// Every run name any module of the file declares.
    pub fn run_names(&self) -> impl Iterator<Item = &str> {
        self.modules.iter().flat_map(|m| m.runs.iter().map(|r| r.name.as_str()))
    }
}

/// Why a file could not be indexed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexError {
    /// 1-based line the problem was detected at.
    pub line: usize,
    /// What was wrong.
    pub reason: &'static str,
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.reason)
    }
}

impl std::error::Error for IndexError {}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Str(String),
    Open,
    Close,
    Punct(char),
}

fn lex(src: &str) -> Result<Vec<(Tok, usize)>, IndexError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut line = 1;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            c if c.is_whitespace() => i += 1,
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                let start = line;
                i += 2;
                loop {
                    match chars.get(i) {
                        None => {
                            return Err(IndexError { line: start, reason: "unterminated comment" });
                        }
                        Some('*') if chars.get(i + 1) == Some(&'/') => {
                            i += 2;
                            break;
                        }
                        Some('\n') => {
                            line += 1;
                            i += 1;
                        }
                        Some(_) => i += 1,
                    }
                }
            }
            '"' => {
                let start = line;
                let mut text = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => {
                            return Err(IndexError { line: start, reason: "unterminated string" });
                        }
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        Some('\\') => {
                            if let Some(&next) = chars.get(i + 1) {
                                text.push(next);
                            }
                            i += 2;
                        }
                        Some(&ch) => {
                            if ch == '\n' {
                                line += 1;
                            }
                            text.push(ch);
                            i += 1;
                        }
                    }
                }
                out.push((Tok::Str(text), start));
            }
            '{' => {
                out.push((Tok::Open, line));
                i += 1;
            }
            '}' => {
                out.push((Tok::Close, line));
                i += 1;
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                out.push((Tok::Ident(chars[start..i].iter().collect()), line));
            }
            other => {
                out.push((Tok::Punct(other), line));
                i += 1;
            }
        }
    }
    Ok(out)
}

/// Index one spec file's source.
pub fn index(src: &str) -> Result<QntIndex, IndexError> {
    let toks = lex(src)?;
    let mut idx = QntIndex::default();
    let mut depth = 0usize;
    let mut current: Option<QntModule> = None;
    let ident = |k: usize| match toks.get(k) {
        Some((Tok::Ident(s), _)) => Some(s.as_str()),
        _ => None,
    };
    let mut k = 0;
    while k < toks.len() {
        let (tok, line) = &toks[k];
        match tok {
            Tok::Open => depth += 1,
            Tok::Close => {
                depth = depth
                    .checked_sub(1)
                    .ok_or(IndexError { line: *line, reason: "unbalanced `}`" })?;
                if depth == 0
                    && let Some(module) = current.take()
                {
                    idx.modules.push(module);
                }
            }
            Tok::Ident(word) if depth == 0 && word == "module" => {
                if let (Some(name), Some((Tok::Open, _))) = (ident(k + 1), toks.get(k + 2)) {
                    current = Some(QntModule {
                        name: name.to_owned(),
                        line: *line,
                        step: false,
                        runs: Vec::new(),
                    });
                    depth = 1;
                    k += 3;
                    continue;
                }
            }
            Tok::Ident(word) if depth == 1 => {
                if let Some(module) = current.as_mut() {
                    match word.as_str() {
                        "action"
                            if ident(k + 1) == Some("step")
                                && matches!(toks.get(k + 2), Some((Tok::Punct('=' | ':'), _))) =>
                        {
                            module.step = true;
                        }
                        "run" => {
                            if let Some(name) = ident(k + 1) {
                                module.runs.push(QntRun { name: name.to_owned(), line: *line });
                            }
                        }
                        "from" => {
                            if let Some((Tok::Str(path), _)) = toks.get(k + 1) {
                                idx.imports.push(path.clone());
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        k += 1;
    }
    if depth != 0 {
        let line = toks.last().map_or(1, |(_, l)| *l);
        return Err(IndexError { line, reason: "unbalanced `{`" });
    }
    Ok(idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = r#"
// A comment mentioning `module fake {` and `run ghost = x`.
module owner {
  /* block: action step = nothing
     run alsoGhost = y */
  import prefix_landing.* from "./prefix_landing"
  var n: int
  action init = n' = 0
  action step = any { n' = n + 1, all { n' = n } }
  pure def label = "run notARun = {"
  run happyPath = init.then(step).expect(n == 1)
  run crashThenRecover: bool = init.then(step)
}
module owner_ok { import owner(OFF = Set()).* from "./owner" }
module defs { pure def f(x: int): int = { x + 1 } }
"#;

    #[test]
    fn indexes_steps_runs_and_imports_skipping_comments_and_strings() {
        let idx = index(SPEC).expect("indexes");
        let names: Vec<&str> = idx.modules.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["owner", "owner_ok", "defs"]);
        let owner = &idx.modules[0];
        assert!(owner.step);
        assert_eq!(owner.line, 3);
        assert_eq!(
            owner.runs,
            [
                QntRun { name: "happyPath".into(), line: 11 },
                QntRun { name: "crashThenRecover".into(), line: 12 },
            ]
        );
        // An instance that imports the stateful module declares no step.
        assert!(!idx.modules[1].step && idx.modules[1].runs.is_empty());
        assert!(!idx.modules[2].step);
        assert_eq!(idx.imports, ["./prefix_landing", "./owner"]);
        assert_eq!(idx.run_names().collect::<Vec<_>>(), ["happyPath", "crashThenRecover"]);
    }

    #[test]
    fn a_step_nested_inside_an_expression_is_not_a_declaration() {
        let idx = index("module m { val x = { action step = 1 } }").expect("indexes");
        assert!(!idx.modules[0].step);
        // `action stepper` and a parameterised action named step-like are not
        // the step relation.
        let idx = index("module m { action stepper = true  action step2 = true }").expect("ok");
        assert!(!idx.modules[0].step);
    }

    #[test]
    fn fails_closed_on_malformed_source() {
        assert_eq!(index("module m { /* open").map_err(|e| e.reason), Err("unterminated comment"));
        assert_eq!(
            index("module m { val s = \"open }").map_err(|e| e.reason),
            Err("unterminated string")
        );
        assert_eq!(index("module m { ").map_err(|e| e.reason), Err("unbalanced `{`"));
        assert_eq!(index("module m { } }").map_err(|e| e.reason), Err("unbalanced `}`"));
    }
}
