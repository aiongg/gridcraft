//! AutoFit must not trust UI shaping measurements that stop at the global cell budget.

use egui::{Color32, Context, FontId, RawInput};
use gridcraft_engine::Session;
use gridcraft_ui_egui::{SheetApp, theme};
use serde_json::json;

fn initialized_app(session: Session) -> (SheetApp, Context) {
    let mut app = SheetApp::new(session, Default::default());
    let context = Context::default();
    // Font binding can schedule one additional activation pass when the background catalog is ready.
    for _ in 0..12 {
        let mut output = context.run_ui(RawInput::default(), |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        output.textures_delta.clear();
    }
    (app, context)
}

fn app_with_column_values(cell_count: usize, wide_last_cell: Option<String>) -> (SheetApp, Context) {
    let mut session = Session::new();
    session.new_workbook();
    let mut values = vec![json!(["i"]); cell_count];
    if let Some(wide_last_cell) = wide_last_cell
        && let Some(last) = values.last_mut()
    {
        *last = json!([wide_last_cell]);
    }
    session.execute("range.setValues", json!({"range": "A1", "values": values})).unwrap();
    session.execute("home.columnWidth", json!({"cols": "A:A", "width": 64.0})).unwrap();
    initialized_app(session)
}

fn measured_width(app: &SheetApp, context: &Context, col: u32, text: &str) -> f32 {
    let doc = app.session.active().unwrap();
    let sheet = doc.wb.active().unwrap();
    let cell = gridcraft_engine::core::CellRef::new(0, col);
    let style = doc.wb.styles.get(sheet.style_id(cell));
    let font_size = if style.font.size.is_finite() { style.font.size.clamp(1.0, 768.0) } else { 11.0 };
    let font_id = FontId::new(font_size * 96.0 / 72.0, theme::cell_family(context, &style.font.name, style.font.bold, style.font.italic));
    context.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), font_id, Color32::BLACK).size().x)
        + 6.0
        + style.align.indent as f32 * 9.0
        + sheet.filter_button_space(cell)
}

#[test]
fn autofit_falls_back_when_the_column_exceeds_the_measurement_budget() {
    let (mut app, _) = app_with_column_values(100_001, Some("W".repeat(180)));

    app.run("home.autofitColumnWidth", json!({"cols": "A:A"})).unwrap();

    assert_eq!(app.session.active().unwrap().wb.active().unwrap().col_width(0), 1785.0);
}

#[test]
fn autofit_uses_ui_measurement_at_the_exact_budget_boundary() {
    let text = "W".repeat(20);
    let (mut app, context) = app_with_column_values(100_000, Some(text.clone()));
    let expected = measured_width(&app, &context, 0, &text).ceil();

    app.run("home.autofitColumnWidth", json!({"cols": "A:A"})).unwrap();

    let width = app.session.active().unwrap().wb.active().unwrap().col_width(0);
    assert_eq!(width, expected, "the fully measured last cell should determine the width");
}

#[test]
fn autofit_keeps_completed_columns_when_a_later_column_exhausts_the_budget() {
    let mut session = Session::new();
    session.new_workbook();
    let mut values = vec![json!(["previous column shaped width", "i"])];
    values.extend((1..100_001).map(|_| json!([null, "i"])));
    if let Some(last) = values.last_mut() {
        *last = json!([null, "W".repeat(180)]);
    }
    session.execute("range.setValues", json!({"range": "A1", "values": values})).unwrap();
    session.execute("home.columnWidth", json!({"cols": "A:B", "width": 64.0})).unwrap();
    let (mut app, context) = initialized_app(session);
    let expected_a = measured_width(&app, &context, 0, "previous column shaped width").ceil();

    app.run("home.autofitColumnWidth", json!({"cols": "A:B"})).unwrap();

    let sheet = app.session.active().unwrap().wb.active().unwrap();
    assert_eq!(sheet.col_width(0), expected_a, "the completed first column should keep its shaped width");
    assert_eq!(sheet.col_width(1), 1785.0, "the incomplete second column should use engine fallback");
}
