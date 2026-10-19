//! DOT (Graphviz) → mol translator: flowchart subset.
//!
//! Supported: `digraph`/`graph` headers (`rankdir=LR|TD` respected), node statements
//! (`a;`, `a [label="x"]`), edge chains (`a -> b -> c;`, `--` for undirected
//! `graph`), per-edge `[label="…"]`, quoted ids/labels, `//`/`#` comments.
//! Subgraphs, `node`/`edge`/`graph[...]` attribute statements are warn-and-skip.

use crate::lexer::is_word_char;

/// The translation result: mol text plus non-fatal notes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotConversion {
    pub mol: String,
    pub warnings: Vec<String>,
}

/// Translate a DOT digraph into a mol flow block.
pub fn import_dot(input: &str) -> Result<DotConversion, String> {
    let mut warnings = Vec::new();
    let statements = split_statements(input)?;
    if statements.is_empty() {
        return Err("no DOT statements found".into());
    }

    let header = statements[0].trim().to_lowercase();
    if !(header.starts_with("digraph") || header.starts_with("graph")) {
        return Err(format!(
            "expected a `digraph`/`graph` header, found `{}`",
            statements[0]
        ));
    }
    let undirected = header.starts_with("graph");
    let arrow = if undirected { "--" } else { "->" };
    let mut direction = "TD".to_owned();

    let mut mol = "flow \"DOT import\" {\n".to_owned();
    let mut declared: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for statement in &statements[1..] {
        if let Some(value) = statement
            .strip_prefix("rankdir=")
            .map(str::trim)
            .map(str::to_owned)
        {
            direction = value.to_uppercase();
            continue;
        }
        if skip_statement(statement, &mut warnings) {
            continue;
        }
        let refs = split_chain(statement, arrow);
        if refs.len() < 2 {
            let (id, label) = node_ref(statement);
            if id.is_empty() {
                warnings.push(format!("statement `{statement}` skipped (untranslatable)"));
                continue;
            }
            if !declared.insert(id.clone()) {
                continue;
            }
            if let Some(label) = label {
                mol.push_str(&format!("  {id}[{label}]\n"));
            }
            continue;
        }
        let parsed: Vec<(String, Option<String>)> =
            refs.iter().map(|reference| node_ref(reference)).collect();
        let (Some(first), Some(last)) = (parsed.first(), parsed.last()) else {
            warnings.push(format!("edge `{statement}` skipped (empty endpoint)"));
            continue;
        };
        // The trailing attr list after the last ref is the edge's label in DOT.
        let edge_label = last.1.clone();
        for (count, (id, label)) in parsed.iter().enumerate() {
            if count + 1 == parsed.len() {
                continue;
            }
            if let Some(label) = label {
                mol.push_str(&format!("  {id}[{label}]\n"));
            }
        }
        for pair in parsed.windows(2) {
            mol.push_str(&format!("  {} --> {}\n", pair[0].0, pair[1].0));
        }
        if let Some(edge_label) = edge_label
            && !edge_label.is_empty()
            && declared.insert(last.0.clone())
        {
            // Edge labels attach to the target node's declaration (wave-2 syntax
            // places edge labels inline; node-attached keeps the subset small).
            mol.push_str(&format!("  {}[{}]\n", last.0, edge_label));
        }
        let _ = &first.1;
        let _ = declared;
    }

    mol.push_str(&format!("  direction: {direction}\n"));
    mol.push('}');
    Ok(DotConversion { mol, warnings })
}

fn skip_statement(statement: &str, warnings: &mut Vec<String>) -> bool {
    let trimmed = statement.trim();
    for keyword in ["subgraph", "node ", "edge ", "graph ", "style ", "class "] {
        if trimmed.starts_with(keyword) {
            warnings.push(format!("statement `{trimmed}` skipped (unsupported)"));
            return true;
        }
    }
    false
}

/// Split the source into statements inside the graph body (top level, quote-aware).
/// Comments (`//`, `#`, `/* */`) are stripped first.
fn split_statements(input: &str) -> Result<Vec<String>, String> {
    let stripped = strip_comments(input);
    let open = stripped.find('{').ok_or("no graph body `{` found")?;
    let close = stripped.rfind('}').ok_or("no graph body `}` found")?;
    if close < open {
        return Err("graph body braces are inverted".into());
    }

    let mut out: Vec<String> = Vec::new();
    let header = stripped[..open].trim().to_owned();
    if !header.is_empty() {
        out.push(header);
    }

    let mut current = String::new();
    let mut in_quotes = false;
    for ch in stripped[open + 1..close].chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            ';' if !in_quotes => {
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_owned());
                }
                current.clear();
            }
            '\n' | '\t' if !in_quotes => current.push(' '),
            _ => current.push(ch),
        }
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_owned());
    }
    Ok(out)
}

fn strip_comments(input: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '/' && chars.get(i + 1) == Some(&'/')
            || chars[i] == '#' && i == 0
            || chars[i] == '#' && chars.get(i - 1) == Some(&'\n')
        {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(chars.len());
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Split an edge chain on the arrow token (quote-aware, index scan).
fn split_chain(statement: &str, arrow: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let chars: Vec<char> = statement.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
                i += 1;
            }
            '-' if !in_quotes && statement[i..].starts_with(arrow) => {
                let trimmed = current.trim().to_owned();
                if !trimmed.is_empty() {
                    refs.push(trimmed);
                }
                current.clear();
                i += arrow.len();
            }
            _ => {
                current.push(ch);
                i += 1;
            }
        }
    }
    let trimmed = current.trim().to_owned();
    if !trimmed.is_empty() {
        refs.push(trimmed);
    }
    refs
}

/// `id` / `id [label="…", …]` → (normalized id, label).
fn node_ref(reference: &str) -> (String, Option<String>) {
    let reference = reference.trim();
    let (id_raw, attrs) = match reference.find('[') {
        Some(index) if reference.ends_with(']') => (
            reference[..index].trim(),
            Some(&reference[index + 1..reference.len() - 1]),
        ),
        _ => (reference, None),
    };
    let id = normalize_id(id_raw);
    let label = attrs.and_then(|attrs| attr_value(attrs, "label"));
    (id, label)
}

fn normalize_id(raw: &str) -> String {
    let raw = raw.trim().trim_matches('"').trim();
    if raw.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for ch in raw.chars() {
        if is_word_char(ch) {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}

fn attr_value(attrs: &str, key: &str) -> Option<String> {
    let marker = format!("{key}=");
    let index = attrs.find(&marker)?;
    let rest = attrs[index + marker.len()..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let close = rest.find('"')?;
    Some(rest[..close].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digraph_chain_with_labels_translates() -> Result<(), Box<dyn std::error::Error>> {
        let conversion = import_dot(
            "digraph G {\n  rankdir=LR;\n  a [label=\"Start\"];\n  a -> b [label=\"go\"];\n  b -> c;\n}\n",
        )?;
        assert!(conversion.mol.contains("direction: LR"));
        assert!(conversion.mol.contains("a[Start]"));
        assert!(conversion.mol.contains("a --> b"));
        assert!(conversion.mol.contains("b --> c"));
        Ok(())
    }

    #[test]
    fn undirected_graph_uses_dash_edges() -> Result<(), Box<dyn std::error::Error>> {
        let conversion = import_dot("graph G {\n  x -- y;\n}\n")?;
        assert!(
            conversion.mol.contains("x --> y"),
            "mol edges normalize to -->"
        );
        Ok(())
    }

    #[test]
    fn comments_and_subgraphs_warn_and_skip() -> Result<(), Box<dyn std::error::Error>> {
        let conversion = import_dot(
            "digraph G {\n  // comment\n  a -> b;\n  subgraph cluster_x { c -> d; }\n  node [shape=box];\n}\n",
        )?;
        // Subgraph contents (edge `c --> d`) must not leak into the flow block.
        assert!(!conversion.mol.contains("c --> d"));
        assert!(
            conversion
                .warnings
                .iter()
                .any(|warning| warning.contains("subgraph"))
        );
        Ok(())
    }

    #[test]
    fn non_dot_input_fails_loudly() {
        assert!(import_dot("sequenceDiagram\n a->>b").is_err());
        assert!(import_dot("").is_err());
    }
}
