//! Canonical printer: AST → mol text. `parse(print(doc))` yields an equal AST.

use crate::ast::{BlockKind, MolBlock, MolDocument, MolLine};

/// Render a document in canonical form (blank line between blocks).
pub fn print(document: &MolDocument) -> String {
    let mut out = String::new();
    for (index, block) in document.blocks.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&print_block(block));
        out.push('\n');
    }
    out
}

fn print_block(block: &MolBlock) -> String {
    print_block_inner(block)
}

fn print_block_inner(block: &MolBlock) -> String {
    let mut out = String::new();
    out.push_str(block.kind.keyword());
    if let Some(title) = &block.title {
        out.push_str(" \"");
        out.push_str(title);
        out.push('"');
    }
    out.push_str(" {\n");
    if block.direction == crate::ast::Direction::TopDown && block.kind != BlockKind::Mind {
        out.push_str("  direction: TD\n");
    }
    for line in &block.lines {
        out.push_str("  ");
        out.push_str(&print_line(line, block.kind));
        out.push('\n');
    }
    out.push('}');
    out
}

fn print_line(line: &MolLine, kind: BlockKind) -> String {
    match line {
        MolLine::NodeDecl { id, label } if label.is_empty() => id.clone(),
        MolLine::NodeDecl { id, label } => format!("{id}[{label}]"),
        MolLine::Edge {
            from,
            to,
            label: None,
        } => {
            format!("{} --> {}", print_node(from), print_node(to))
        }
        MolLine::Edge {
            from,
            to,
            label: Some(label),
        } => {
            format!("{} --> {} : {}", print_node(from), print_node(to), label)
        }
        MolLine::MindLink { chain } => chain.join(" > "),
        MolLine::SeqMsg {
            from,
            to,
            dashed,
            label,
        } => {
            let arrow = if *dashed { "-->" } else { "->" };
            match label {
                Some(text) => format!("{from} {arrow} {to} : {text}"),
                None => format!("{from} {arrow} {to}"),
            }
        }
        // Pie grammar takes quoted keys (`"A" : 30`); every other Members kind
        // takes bare words.
        MolLine::Members { id, text } if kind == BlockKind::Pie => {
            format!("\"{id}\": {text}")
        }
        MolLine::Members { id, text } => format!("{id}: {text}"),
        MolLine::GitOp { op, arg } if arg.is_empty() => op.keyword().to_owned(),
        MolLine::GitOp { op, arg } => format!("{} {arg}", op.keyword()),
        MolLine::Direction {
            direction: crate::ast::Direction::TopDown,
        } => "direction: TD".into(),
        MolLine::Direction {
            direction: crate::ast::Direction::LeftRight,
        } => "direction: LR".into(),
    }
}

fn print_node(node: &crate::ast::MolNode) -> String {
    match &node.label {
        Some(label) => format!("{}[{}]", node.id, label),
        None => node.id.clone(),
    }
}
