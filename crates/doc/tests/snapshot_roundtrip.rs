//! Property: arbitrary element sets survive the snapshot → restore round trip
//! with identical content.

use e6_doc::BoardDoc;
use e6_doc::ElementSpec;
use e6_scene::ElementKind;
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

fn spec_strategy() -> BoxedStrategy<ElementSpec> {
    let kind = proptest::sample::select(vec![
        ElementKind::Rect,
        ElementKind::Ellipse,
        ElementKind::Text,
        ElementKind::Sticky,
    ]);
    (
        kind,
        0.0_f64..400.0,
        0.0_f64..400.0,
        20.0_f64..200.0,
        20.0_f64..200.0,
        "[a-z0-9 ]{0,24}",
    )
        .prop_map(|(kind, x, y, w, h, text)| {
            ElementSpec::simple(
                kind,
                e6_scene::Aabb::from_min_size(
                    e6_scene::Point::new(x as f32, y as f32),
                    e6_scene::Size::new(w as f32, h as f32),
                ),
                0xFF22_4488,
            )
            .with_text(text, 24.0)
        })
        .boxed()
}

proptest! {
    #[test]
    fn snapshots_round_trip_arbitrary_boards(specs in proptest::collection::vec(spec_strategy(), 0..25)) {
        let doc = BoardDoc::new();
        for spec in &specs {
            doc.insert_element(spec)
                .map_err(|err| TestCaseError::fail(format!("insert failed: {err}")))?;
        }
        doc.commit();

        let snapshot = doc
            .snapshot()
            .map_err(|err| TestCaseError::fail(format!("snapshot failed: {err}")))?;
        let restored = BoardDoc::from_snapshot(&snapshot)
            .map_err(|err| TestCaseError::fail(format!("restore failed: {err}")))?;

        let mut original = doc.elements();
        let mut restored_elements = restored.elements();
        let sort_key = |elements: &mut Vec<e6_scene::Element>| {
            elements.sort_by_key(|element| element.id.raw());
        };
        sort_key(&mut original);
        sort_key(&mut restored_elements);
        prop_assert_eq!(original.len(), restored_elements.len());
        for (before, after) in original.iter().zip(restored_elements.iter()) {
            prop_assert_eq!(before.id, after.id);
            prop_assert_eq!(before.kind, after.kind);
            prop_assert_eq!(before.bounds, after.bounds);
            prop_assert_eq!(before.text.as_str(), after.text.as_str());
            prop_assert_eq!(before.color, after.color);
        }
    }
}
