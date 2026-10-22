//! Offscreen regression render for Thai shaping, wrapping, and font fallback.
//!
//! Run with `cargo run -p gridcraft-ui-egui --example thai_shaping_snapshot -- target/thai-text-shaping.png`.

use gridcraft_engine::Session;
use gridcraft_ui_egui::{SheetApp, fonts};
use serde_json::json;
use skrifa::MetadataProvider;

fn has_thai(face: &fonts::Face) -> bool {
    std::fs::read(&face.path)
        .is_ok_and(|bytes| skrifa::FontRef::from_index(&bytes, face.index).is_ok_and(|font| font.charmap().map('\u{0E20}').is_some()))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.first().map(String::as_str).unwrap_or("target/thai-text-shaping.png");
    let selected_family = fonts::catalog()
        .iter()
        .find(|face| !face.italic && (face.weight - 400.0).abs() < 1.0 && !has_thai(face))
        .map(|face| face.family.clone())
        .unwrap_or_else(|| "Calibri".to_owned());
    let thai_fallback = fonts::catalog().iter().any(has_thai);

    let mut session = Session::new();
    session.new_workbook();
    let long = format!("{} English ", "\u{0E02}\u{0E49}\u{0E2D}\u{0E04}\u{0E27}\u{0E32}\u{0E21}\u{0E22}\u{0E32}\u{0E27}").repeat(5);
    let values = [
        (
            "A1",
            "\u{0E20}\u{0E32}\u{0E29}\u{0E32}\u{0E44}\u{0E17}\u{0E22} \u{0E19}\u{0E49}\u{0E33} \u{0E01}\u{0E38}\u{0E49}\u{0E07} \u{0E1C}\u{0E39}\u{0E49}\u{0E43}\u{0E0A}\u{0E49} \u{0E40}\u{0E01}\u{0E49}\u{0E32}\u{0E2D}\u{0E35}\u{0E49}",
        ),
        ("A2", "\u{0E1B} \u{0E1B} \u{0E0D} \u{0E10}"),
        ("A3", "\u{0E22}\u{0E2D}\u{0E14}\u{0E23}\u{0E27}\u{0E21} 1,234.50 \u{0E1A}\u{0E32}\u{0E17}"),
        ("A4", "English \u{0E44}\u{0E17}\u{0E22} mixed words\n\u{0E22}\u{0E2D}\u{0E14}\u{0E23}\u{0E27}\u{0E21} 1,234.50 \u{0E1A}\u{0E32}\u{0E17}"),
        ("A5", &long),
        ("B1", "\u{0E19}\u{0E49}\u{0E33}"),
    ];
    for (cell, input) in values {
        if let Err(error) = session.execute("cell.set", json!({"cell":cell, "input":input})) {
            eprintln!("cell.set {cell}: {error}");
            return;
        }
    }
    for (command, params) in [
        ("home.formatCells", json!({"range":"A1:A5", "style":{"font":{"name":selected_family,"size":12.0}, "align":{"wrap":true, "v":"Top"}}})),
        ("home.formatCells", json!({"range":"B1", "style":{"font":{"name":selected_family,"size":12.0}, "align":{"wrap":true, "v":"Top"}}})),
        ("home.bold", json!({"range":"A2"})),
        ("home.italic", json!({"range":"A3"})),
        ("home.bold", json!({"range":"A4"})),
        ("home.italic", json!({"range":"A4"})),
        ("home.columnWidth", json!({"cols":"A:A", "width":165.0})),
        ("home.columnWidth", json!({"cols":"B:B", "width":19.0})),
        ("home.rowHeight", json!({"rows":"1:5", "height":18.0})),
    ] {
        if let Err(error) = session.execute(command, params) {
            eprintln!("{command}: {error}");
            return;
        }
    }

    let mut harness =
        egui_kittest::Harness::builder().with_size(egui::vec2(1120.0, 650.0)).with_pixels_per_point(2.0).with_max_steps(12).wgpu().build_ui_state(
            |ui, app: &mut SheetApp| {
                let context = ui.ctx().clone();
                app.logic(&context);
                app.ui(ui);
            },
            SheetApp::new(session, Default::default()),
        );
    harness.run_steps(8);
    harness.state_mut().run("home.autofitRowHeight", json!({"rows":"1:5"})).expect("autofit rendered Thai rows");
    harness.run_steps(3);
    let image = match harness.render() {
        Ok(image) => image,
        Err(error) => {
            eprintln!("offscreen rendering is unavailable: {error}");
            return;
        }
    };
    if let Err(error) = image.save(out) {
        eprintln!("could not save {out}: {error}");
        return;
    }
    eprintln!("wrote {out} ({}x{}), main font {selected_family:?}, Thai fallback installed: {thai_fallback}", image.width(), image.height());
}
