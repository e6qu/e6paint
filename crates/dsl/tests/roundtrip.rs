//! Canonical-printer round-trip properties and parser robustness (AGENTS testing
//! pyramid: property tests over the dsl).
//!
//! - `parse` never panics, on any input whatsoever.
//! - Anything that parses survives `print` → `parse` unchanged, and the printed
//!   form is a fixpoint (printing twice equals printing once).

use e6_dsl::{parse, print};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

proptest! {
    /// No input, however adversarial, may panic the parser.
    #[test]
    fn parse_never_panics(src in any::<String>()) {
        let _ = parse(&src);
    }

    /// Whatever random input parses, the printed form re-parses to the same AST
    /// and printing is byte-stable.
    #[test]
    fn printed_form_is_canonical(src in any::<String>()) {
        if let Ok(doc) = parse(&src) {
            let once = print(&doc);
            let reparsed = parse(&once).map_err(|errors| {
                TestCaseError::fail(format!(
                    "printer output did not re-parse:\n{once}\n{errors:?}"
                ))
            })?;
            prop_assert_eq!(print(&reparsed), once);
            prop_assert_eq!(reparsed, doc);
        }
    }

    /// Grammar-composed sources: every piece is real mol syntax; when the mix is
    /// valid for the drawn kind, the document must round-trip.
    #[test]
    fn grammar_composed_sources_round_trip(src in mol_source()) {
        let _ = parse(&src);
        if let Ok(doc) = parse(&src) {
            let once = print(&doc);
            let reparsed = parse(&once).map_err(|errors| {
                TestCaseError::fail(format!(
                    "printer output did not re-parse:\n{once}\n{errors:?}"
                ))
            })?;
            prop_assert_eq!(print(&reparsed), once);
            prop_assert_eq!(reparsed, doc);
        }
    }
}

fn mol_source() -> BoxedStrategy<String> {
    let header = prop_oneof![
        Just("flow \"F\" {"),
        Just("state \"S\" {"),
        Just("mind \"M\" {"),
        Just("seq \"Q\" {"),
        Just("er \"E\" {"),
        Just("class \"C\" {"),
        Just("git \"G\" {"),
        Just("timeline \"T\" {"),
        Just("pie \"P\" {"),
        Just("gantt \"A\" {"),
    ];
    let piece = prop_oneof![
        Just("a --> b"),
        Just("a[Start] --> b[End] : goes"),
        Just("n1[Node one]"),
        Just("direction: TD"),
        Just("x > y > z"),
        Just("alice -> bob : hello"),
        Just("alice --> bob : bye"),
        Just("CUSTOMER: id PK, email"),
        Just("Animal <|-- Dog"),
        Just("commit init"),
        Just("branch dev"),
        Just("checkout dev"),
        Just("merge dev"),
        Just("Design : 2024-01-01, 5d"),
        Just("\"Slice\" : 30"),
    ];
    (header, prop::collection::vec(piece, 1..6))
        .prop_map(|(header, lines)| format!("{header}\n{}\n}}\n", lines.join("\n")))
        .boxed()
}

/// One verified-good source per block kind; pins printer canonicity for every
/// grammar family, no generator luck involved.
#[test]
fn corpus_round_trips_per_kind() -> Result<(), Box<dyn std::error::Error>> {
    let corpus = [
        "flow \"F\" {\n  a --> b\n  a --> c\n}\n",
        "state \"Machine\" {\n  direction: TD\n  idle --> running\n}\n",
        "mind \"Roadmap\" {\n  e6paint > collab > relay\n}\n",
        "seq \"Handshake\" {\n  alice -> bob : SYN\n  bob --> alice : SYN-ACK\n}\n",
        "er \"Shop\" {\n  CUSTOMER: id PK, email\n  ORDER: total\n  CUSTOMER --> ORDER : places\n}\n",
        "class \"Shapes\" {\n  Animal: +name\n  Dog: +bark()\n  Animal <|-- Dog\n}\n",
        "timeline \"Plan\" {\n  Q1: design, build\n}\n",
        "pie \"Sales\" {\n  \"A\" : 30\n  \"B\" : 10\n}\n",
        "gantt \"Plan\" {\n  Design : 2024-01-01, 5d\n  Build : 2024-01-06, 10d\n}\n",
        "git \"History\" {\n  commit init\n  branch dev\n  checkout dev\n  merge dev\n}\n",
    ];
    for source in corpus {
        let doc = parse(source)
            .map_err(|errors| format!("corpus source failed to parse:\n{source}{errors:?}"))?;
        let once = print(&doc);
        let reparsed = parse(&once)
            .map_err(|errors| format!("printer output did not re-parse:\n{once}{errors:?}"))?;
        assert_eq!(reparsed, doc, "round-trip changed the AST for:\n{source}");
        assert_eq!(
            print(&reparsed),
            once,
            "print is not a fixpoint for:\n{source}"
        );
    }
    Ok(())
}
