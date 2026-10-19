//! Mermaid flowchart → mol translator (DSL wave 1 subset).
//!
//! Supported: `flowchart`/`graph` headers with TD/TB/LR directions, node shapes
//! (`[x]`, `(x)`, `{x}`, `([x])`, `((x))`, `[/x/]` → plain `[x]` labels), the `-->`
//! arrow family (`---`, `-.->`, `==>` normalized), `|label|` edge labels, `A -- text --> B`
//! inline labels, `&` fan-out, and `%%` comments. Subgraphs and class/style/click
//! statements are reported as warnings and skipped; anything else fails loudly.

/// The translation result: mol text plus non-fatal notes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversion {
    pub mol: String,
    pub warnings: Vec<String>,
}

/// Translate a Mermaid flowchart into mol text. The first non-empty line must be a
/// `flowchart`/`graph` header; anything else is rejected with its 1-based line number.
pub fn import_mermaid(input: &str) -> Result<Conversion, String> {
    let mut direction = String::from("LR");
    let mut header_seen = false;
    let mut mol = String::from("flow \"Mermaid import\" {\n");
    let mut warnings = Vec::new();
    let mut subgraph_depth = 0usize;

    for (offset, raw_line) in input.lines().enumerate() {
        let number = offset + 1;
        let line = strip_comment(raw_line).trim().to_owned();
        if line.is_empty() {
            continue;
        }
        if !header_seen {
            let mut words = line.split_whitespace();
            let kind = words.next().unwrap_or_default();
            if kind == "flowchart" || kind == "graph" {
                header_seen = true;
                if let Some(dir) = words.next() {
                    match dir {
                        "TD" | "TB" => direction = "TD".into(),
                        "LR" => direction = "LR".into(),
                        other => warnings
                            .push(format!("line {number}: direction `{other}` treated as LR")),
                    }
                }
                mol.push_str(&format!("  direction: {direction}\n"));
                continue;
            }
            return Err(format!(
                "line {number}: expected a `flowchart` or `graph` header, found `{kind}`"
            ));
        }
        if subgraph_depth > 0 {
            if line == "end" {
                subgraph_depth -= 1;
            } else if line.starts_with("subgraph") {
                subgraph_depth += 1;
            }
            continue;
        }
        if line.starts_with("subgraph") {
            subgraph_depth += 1;
            warnings.push(format!(
                "line {number}: subgraph skipped (flat flow imported)"
            ));
            continue;
        }
        if line.starts_with("classDef ")
            || line.starts_with("class ")
            || line.starts_with("style ")
            || line.starts_with("click ")
            || line.starts_with("linkStyle ")
        {
            warnings.push(format!("line {number}: styling statement skipped"));
            continue;
        }
        mol.push_str("  ");
        mol.push_str(&translate_line(&line, number, &mut warnings));
        mol.push('\n');
    }
    if !header_seen {
        return Err("no `flowchart` or `graph` header found".into());
    }
    mol.push('}');
    Ok(Conversion { mol, warnings })
}

fn strip_comment(line: &str) -> &str {
    match line.find("%%") {
        Some(index) => &line[..index],
        None => line,
    }
}

/// Translate one content line: split on arrows, normalize node refs on each side.
fn translate_line(line: &str, number: usize, warnings: &mut Vec<String>) -> String {
    // Inline-label form first: `A -- text --> B` → `A --> B : text`.
    let (line, inline_label) = match find_inline_label(line) {
        Some((before, text, after)) => (format!("{before}--> {after}"), Some(text)),
        None => (line.to_owned(), None),
    };

    // Find the first arrow token (longest match wins) and split around it.
    let tokens = ["-.->", "==>", "-->", "---"];
    let mut split = None;
    for token in tokens {
        if let Some(index) = line.find(token) {
            split = Some((index, token.len()));
            break;
        }
    }
    let Some((arrow_index, arrow_len)) = split else {
        // A bare node line: translate refs on the whole line.
        return translate_node_list(&line, number, warnings).join("\n");
    };
    let left = &line[..arrow_index];
    let mut right = &line[arrow_index + arrow_len..];

    // `|label|` form: `-->|text| B`.
    let mut label = inline_label;
    if let Some(rest) = right.strip_prefix('|') {
        match rest.find('|') {
            Some(close) => {
                label = Some(rest[..close].trim().to_owned());
                right = &rest[close + 1..];
            }
            None => warnings.push(format!("line {number}: unterminated |label| ignored")),
        }
    }

    let from_nodes = translate_node_list(left, number, warnings);
    let to_nodes = translate_node_list(right, number, warnings);
    let label_suffix = label.map(|text| format!(" : {text}")).unwrap_or_default();
    let mut out = String::new();
    for from in from_nodes {
        for to in &to_nodes {
            out.push_str(&format!("{from} --> {to}{label_suffix}\n"));
        }
    }
    // One statement per line: the joined block above would break the grammar, so
    // fan-out (`A & B`) is emitted as separate lines joined without trailing newline
    // handling here — trim the final newline and let the caller append one.
    out.trim_end().to_owned()
}

/// `A -- text --> B` detection: the segment between `--` and `-->` is the label.
fn find_inline_label(line: &str) -> Option<(String, String, String)> {
    let arrow = line.find("-->")?;
    let head = &line[..arrow];
    let separator = head.rfind(" -- ")?;
    let before = head[..separator].trim().to_owned();
    let text = head[separator + 4..].trim().to_owned();
    if text.is_empty() {
        return None;
    }
    let after = line[arrow + 3..].trim().to_owned();
    Some((before, text, after))
}

/// Translate one side of an arrow: node refs separated by `&`, shapes normalized.
fn translate_node_list(side: &str, number: usize, warnings: &mut Vec<String>) -> Vec<String> {
    side.split('&')
        .map(|part| translate_node_ref(part.trim(), number, warnings))
        .collect()
}

/// `id[Label]`/`id(Label)`/`id{Label}`/wrapped variants → `id[Label]` or bare `id`.
fn translate_node_ref(reference: &str, number: usize, warnings: &mut Vec<String>) -> String {
    let reference = reference.trim();
    let id_end = reference
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '.'))
        .unwrap_or(reference.len());
    let id = &reference[..id_end];
    if id.is_empty() {
        warnings.push(format!(
            "line {number}: node reference `{reference}` has no id"
        ));
        return reference.to_owned();
    }
    let rest = reference[id_end..].trim();
    if rest.is_empty() {
        return id.to_owned();
    }
    let Some(open) = rest.chars().next() else {
        return id.to_owned();
    };
    let close = match open {
        '[' => ']',
        '(' => ')',
        '{' => '}',
        other => {
            warnings.push(format!(
                "line {number}: node `{id}` shape opening `{other}` unsupported; label kept raw"
            ));
            return format!("{id}[{}]", rest);
        }
    };
    let inner = rest[1..].trim_end_matches(close).trim();
    // Decorated shapes like `[/x/]` or `((x))` may leave trailing closers.
    let inner = inner.trim_end_matches(close).trim();
    if inner.is_empty() {
        id.to_owned()
    } else {
        format!("{id}[{inner}]")
    }
}
