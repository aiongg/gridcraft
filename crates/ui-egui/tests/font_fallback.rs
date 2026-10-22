use egui::{Color32, FontFamily, FontId};
use gridcraft_engine::Session;
use gridcraft_ui_egui::{SheetApp, fonts, theme};
use serde_json::json;
use skrifa::MetadataProvider;

#[test]
fn completed_font_scan_binds_workbook_font_roles_before_rendering() {
    fonts::start_scan();
    let Some(face) = fonts::catalog().iter().find(|face| {
        !face.italic
            && (face.weight - 400.0).abs() < 1.0
            && std::fs::read(&face.path)
                .is_ok_and(|bytes| skrifa::FontRef::from_index(&bytes, face.index).is_ok_and(|font| font.charmap().map('ภ').is_none()))
    }) else {
        return; // A machine without a suitable installed workbook font has nothing to bind.
    };
    let family = face.family.clone();
    let thai_fallback_available = theme::font_definitions().font_data.contains_key("sys-thai");

    let mut session = Session::new();
    session.execute("file.new", json!({})).expect("new workbook");
    session.execute("cell.set", json!({"cell": "A1", "input": "ภาษาไทย น้ำ กุ้ง ผู้ใช้ เก้าอี้ ยอดรวม 1,234.50 บาท"})).expect("set sample");
    session.execute("home.fontName", json!({"range": "A1", "name": family})).expect("set installed font");

    let mut app = SheetApp::new(session, Default::default());
    let context = egui::Context::default();
    for _ in 0..4 {
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        output.textures_delta.clear();
    }

    let role = fonts::role_key(&face.family, false, false);
    assert!(fonts::scan_done(), "font catalog should be complete before workbook fonts bind");
    assert!(
        context.fonts(|fonts| fonts.definitions().families.contains_key(&FontFamily::Name(role.clone().into()))),
        "the workbook's installed font was not rebound in this context after scan completion"
    );

    if thai_fallback_available {
        let glyphs = |text: &str| {
            context.fonts_mut(|font_system| {
                font_system
                    .layout_no_wrap(text.to_owned(), FontId::new(14.0, FontFamily::Name(role.clone().into())), Color32::BLACK)
                    .rows
                    .iter()
                    .flat_map(|row| row.glyphs.iter().filter(|glyph| glyph.chr != ' ').map(|glyph| glyph.uv_rect))
                    .collect::<Vec<_>>()
            })
        };
        let tofu = glyphs("\u{25FB}");
        let rendered = glyphs("ภาษาไทย น้ำ กุ้ง ผู้ใช้ เก้าอี้ ยอดรวม 1,234.50 บาท");
        assert!(!rendered.is_empty(), "sample produced no glyphs");
        assert!(rendered.iter().all(|glyph| !tofu.contains(glyph)), "workbook font did not fall back to the installed Thai face");
    }
}
