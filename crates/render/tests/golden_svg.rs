//! Display-list golden: the SVG export of a fixed fixture scene must stay
//! byte-stable. The committed golden is the reviewable artifact; regenerate with
//! `E6PAINT_UPDATE_GOLDENS=1 cargo test -p e6-render --test golden_svg`.

use e6_render::export_svg;
use e6_scene::{Aabb, Element, ElementId, ElementKind, Point, Size};

fn element(id: u64, kind: ElementKind) -> Element {
    Element::new(
        ElementId::from_raw(id),
        kind,
        Aabb::from_min_size(Point::new(10.0, 10.0), Size::new(50.0, 40.0)),
        0xFF22_4488,
    )
}

fn fixture() -> Vec<Element> {
    vec![
        element(1, ElementKind::Rect),
        {
            let mut text = element(2, ElementKind::Text);
            text.text = "hello <board>".into();
            text.font_size = 24.0;
            text
        },
        {
            let mut connector = element(3, ElementKind::Connector);
            connector.points = vec![Point::new(0.0, 0.0), Point::new(80.0, 60.0)];
            connector.stroke_width = 2.0;
            connector
        },
    ]
}

fn golden_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens/board.svg")
}

#[test]
fn svg_export_matches_golden() -> Result<(), Box<dyn std::error::Error>> {
    let svg = export_svg(&fixture(), "Golden board");
    let path = golden_path();
    if std::env::var_os("E6PAINT_UPDATE_GOLDENS").is_some() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &svg)?;
        eprintln!("golden updated at {}", path.display());
        return Ok(());
    }
    let expected = std::fs::read_to_string(&path).map_err(|err| {
        format!(
            "golden {} missing ({err}); run E6PAINT_UPDATE_GOLDENS=1 cargo test -p e6-render",
            path.display()
        )
    })?;
    assert_eq!(svg, expected, "SVG export drifted from the golden");
    Ok(())
}
