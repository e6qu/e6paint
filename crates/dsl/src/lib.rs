//! The native `mol` DSL: lexer, parser, printer, and the Mermaid/DOT importers.
//!
//! Grammar direction, locked by design: hybrid arrows + blocks —
//!
//! - edges as `a --> b` and `a --> b : edge label`
//! - families and nesting as typed blocks (`flow { }`, `state { }`, `mind { }`, …)
//! - attributes as `key: value` (`direction: LR` / `TD` in wave 1)
//! - mindmaps link with `parent > child`; one statement per line; `//` comments
//!
//! One shared statement/arrow core with thin family ASTs — not fifteen ad-hoc grammars.
//! Expansion is one-way: parse → layout → elements stamped with `(block, node)`
//! provenance; re-apply regenerates. Mermaid and DOT import translate into `mol` and
//! ride the same expansion path.

pub mod ast;
pub mod dot;
pub mod expand;
pub mod lexer;
pub mod mermaid;
pub mod parser;
pub mod print;

pub use ast::{BlockKind, Direction, MolBlock, MolDocument, MolLine, MolNode};
pub use dot::import_dot;
pub use expand::{
    ExpandError, ExpandOptions, ExpandedBlock, ExpandedEdge, ExpandedNode, Expansion, expand,
};
pub use mermaid::{Conversion, import_mermaid};
pub use parser::{ParseError, parse};
pub use print::print;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::MolLine;
    use crate::expand::ExpandedNode;

    #[test]
    fn parses_flow_blocks_with_edges_and_labels() -> Result<(), Vec<ParseError>> {
        let doc = parse(
            "// comment\nflow \"Auth flow\" {\n  start[Start]\n  start --> gate : check\n  gate[Gate]\n}\n",
        )?;
        assert_eq!(doc.blocks.len(), 1);
        let block = &doc.blocks[0];
        assert_eq!(block.kind, BlockKind::Flow);
        assert_eq!(block.title.as_deref(), Some("Auth flow"));
        assert!(block.lines.iter().any(
            |line| matches!(line, MolLine::Edge { label: Some(text), .. } if text == "check")
        ));
        Ok(())
    }

    #[test]
    fn seq_er_class_parse_and_expand() -> Result<(), Box<dyn std::error::Error>> {
        let source = "seq \"Handshake\" {\n  alice -> bob : SYN\n  bob --> alice : SYN-ACK\n  alice -> bob : ACK\n}\n";
        let doc = parse(source).map_err(|errors| format!("parse errors: {errors:?}"))?;
        assert_eq!(doc.blocks[0].kind, BlockKind::Seq);
        let expansion = expand(&doc, &ExpandOptions::default())?;
        let block = &expansion.blocks[0];
        assert_eq!(block.nodes.len(), 2, "alice + bob");
        assert_eq!(block.edges.len(), 3, "three messages");
        assert!(block.edges[1].points[0].y > block.edges[0].points[0].y);

        let er_source = "er \"Shop\" {\n  CUSTOMER: id PK, email\n  ORDER: total\n  CUSTOMER --> ORDER : places\n}\n";
        let doc = parse(er_source).map_err(|errors| format!("{errors:?}"))?;
        let expansion = expand(&doc, &ExpandOptions::default())?;
        let labels: Vec<String> = expansion.blocks[0]
            .nodes
            .iter()
            .map(|node| node.label.clone())
            .collect();
        assert!(
            labels
                .iter()
                .any(|label| label.contains("CUSTOMER") && label.contains("id PK")),
            "labels: {labels:?}"
        );

        let class_source =
            "class \"Shapes\" {\n  Animal: +name\n  Dog: +bark()\n  Animal <|-- Dog\n}\n";
        let doc = parse(class_source).map_err(|errors| format!("{errors:?}"))?;
        let expansion = expand(&doc, &ExpandOptions::default())?;
        assert_eq!(expansion.blocks[0].edges.len(), 1, "inheritance edge");
        Ok(())
    }

    #[test]
    fn dot_import_translates_chains_and_labels() -> Result<(), Box<dyn std::error::Error>> {
        let conversion = import_dot(
            "digraph G {\n  rankdir=LR;\n  a [label=\"Start\"];\n  a -> b [label=\"go\"];\n  b -> c;\n}\n",
        )?;
        assert!(conversion.warnings.is_empty(), "{:?}", conversion.warnings);
        let doc = parse(&conversion.mol).map_err(|errors| format!("{errors:?}"))?;
        assert_eq!(doc.blocks[0].direction, Direction::LeftRight);
        assert!(conversion.mol.contains("a[Start]"));
        assert!(conversion.mol.contains("a --> b"));
        assert!(conversion.mol.contains("b --> c"));

        // Edge label attaches to the target node's declaration (wave-2 subset).
        assert!(conversion.mol.contains("b[go]") || conversion.mol.contains("b[go]"));
        Ok(())
    }

    #[test]
    fn pie_gantt_git_chart_expansions() -> Result<(), Box<dyn std::error::Error>> {
        let pie = parse("pie \"Sales\" {\n  \"A\" : 30\n  \"B\" : 10\n}\n")
            .map_err(|errors| format!("{errors:?}"))?;
        let pie_expansion = expand(&pie, &ExpandOptions::default())?;
        let slices: Vec<&ExpandedNode> = pie_expansion.blocks[0]
            .nodes
            .iter()
            .filter(|node| node.points.len() > 3)
            .collect();
        assert_eq!(slices.len(), 2, "two sector polygons");

        let gantt =
            parse("gantt \"Plan\" {\n  Design : 2024-01-01, 5d\n  Build : 2024-01-06, 10d\n}\n")
                .map_err(|errors| format!("{errors:?}"))?;
        let gantt_expansion = expand(&gantt, &ExpandOptions::default())?;
        assert_eq!(gantt_expansion.blocks[0].nodes.len(), 2);

        let git = parse("git \"History\" {\n  commit init\n  branch dev\n  checkout dev\n  commit feature\n  merge dev\n}\n")
            .map_err(|errors| format!("{errors:?}"))?;
        let git_expansion = expand(&git, &ExpandOptions::default())?;
        assert!(git_expansion.blocks[0].nodes.len() >= 3, "commits placed");
        assert!(
            git_expansion.blocks[0]
                .edges
                .iter()
                .any(|edge| edge.label.as_deref() == Some("merge"))
        );
        Ok(())
    }

    #[test]
    fn malformed_chart_rows_fail_loudly() -> Result<(), Box<dyn std::error::Error>> {
        let bad_row = |err: Option<ExpandError>| err.ok_or("expected expand to fail".to_string());
        let pie = parse("pie \"Sales\" {\n  \"A\" : thirty\n}\n").map_err(|e| format!("{e:?}"))?;
        let err = bad_row(expand(&pie, &ExpandOptions::default()).err())?;
        assert!(
            matches!(err, ExpandError::BadRow { row: 0, .. }),
            "unexpected error: {err:?}"
        );

        let gantt =
            parse("gantt \"Plan\" {\n  Design : someday, 5d\n}\n").map_err(|e| format!("{e:?}"))?;
        let err = bad_row(expand(&gantt, &ExpandOptions::default()).err())?;
        assert!(
            matches!(err, ExpandError::BadRow { row: 0, .. }),
            "unexpected error: {err:?}"
        );

        let gantt = parse("gantt \"Plan\" {\n  Design : 2024-01-01, soon\n}\n")
            .map_err(|e| format!("{e:?}"))?;
        let err = bad_row(expand(&gantt, &ExpandOptions::default()).err())?;
        assert!(
            matches!(err, ExpandError::BadRow { row: 0, .. }),
            "unexpected error: {err:?}"
        );
        Ok(())
    }

    #[test]
    fn mind_links_and_direction_attributes_parse() -> Result<(), Vec<ParseError>> {
        let doc = parse("mind \"Roadmap\" {\n  e6paint > collab > relay\n  e6paint > dsl\n}\n")?;
        let block = &doc.blocks[0];
        assert_eq!(block.kind, BlockKind::Mind);
        assert!(
            block.lines.iter().any(|line| matches!(
                line,
                MolLine::MindLink { chain } if chain.as_slice() == ["e6paint", "collab", "relay"]
            )),
            "lines parsed: {:?}",
            block.lines
        );

        let doc = parse("state \"Machine\" {\n  direction: TD\n  idle --> running\n}\n")?;
        assert_eq!(doc.blocks[0].direction, Direction::TopDown);
        Ok(())
    }

    #[test]
    fn errors_carry_locations_and_recovery_continues() -> Result<(), Box<dyn std::error::Error>> {
        let errors = match parse("flow {\n  a -->\n  b --> c\n  : stray\n}\n") {
            Err(errors) => errors,
            Ok(_) => return Err("broken statements must error".into()),
        };
        assert!(
            errors.len() >= 2,
            "both broken statements reported: {errors:?}"
        );
        assert!(
            errors[0].line >= 2 && errors[0].line <= 3,
            "located: {errors:?}"
        );
        Ok(())
    }

    #[test]
    fn printer_round_trips_the_ast() -> Result<(), Vec<ParseError>> {
        let source = "flow \"Flow\" {\n  direction: TD\n  a[A]\n  a --> b : yes\n}\n\nmind {\n  root > kid\n}\n";
        let doc = parse(source)?;
        let printed = print(&doc);
        assert_eq!(parse(&printed)?, doc, "parse(print(parse(x))) is stable");
        Ok(())
    }

    #[test]
    fn mermaid_flowchart_imports_with_shapes_and_labels() -> Result<(), Box<dyn std::error::Error>>
    {
        let conversion = import_mermaid(
            "flowchart TD\n  %% a comment\n  A[Start] --> B{Choice}\n  B -->|yes| C(db)\n  A -- fallback --> C\n",
        )?;
        let doc = match parse(&conversion.mol) {
            Ok(doc) => doc,
            Err(errors) => return Err(format!("imported mol failed to parse: {errors:?}").into()),
        };
        assert_eq!(doc.blocks.len(), 1);
        assert_eq!(doc.blocks[0].direction, Direction::TopDown);
        for expected in [
            "A[Start] --> B[Choice]",
            "B --> C[db] : yes",
            "A --> C : fallback",
        ] {
            assert!(
                conversion.mol.contains(expected),
                "missing {expected:?} in mol:\n{}",
                conversion.mol
            );
        }
        Ok(())
    }

    #[test]
    fn mermaid_rejects_other_families_and_warns_on_subgraphs()
    -> Result<(), Box<dyn std::error::Error>> {
        assert!(import_mermaid("sequenceDiagram\n  a->>b: hi\n").is_err());
        let conversion = import_mermaid(
            "graph LR\n  subgraph S\n    A --> B\n  end\n  classDef x fill:red\n  A --> C\n",
        )?;
        assert!(
            conversion
                .warnings
                .iter()
                .any(|warning| warning.contains("subgraph skipped"))
        );
        assert!(
            !conversion.mol.contains("B --> C"),
            "subgraph contents skipped"
        );
        Ok(())
    }
}
