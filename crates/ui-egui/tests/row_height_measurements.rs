use egui::{Color32, Context, FontId, RawInput};
use gridcraft_engine::core::{CellRef, RangeRef, Value};
use gridcraft_engine::model::{Cell, HAlign, LineInfo, StyleId, Workbook};
use gridcraft_engine::{DocState, Session};
use gridcraft_ui_egui::{SheetApp, theme};
use serde_json::json;

fn fixture(custom_height: bool, merged: bool) -> Session {
    let mut workbook = Workbook::new();
    let style = workbook.styles.derive(StyleId::DEFAULT, |style| {
        style.font.name = "GridCraft Fixture Sans".into();
        style.font.size = 22.0;
        style.align.wrap = true;
        style.align.h = HAlign::Left;
    });
    let sheet = workbook.sheet_mut(0).expect("default sheet exists");
    sheet.cols.insert(1, LineInfo { size: Some(if merged { 100.0 } else { 110.0 }), ..LineInfo::default() });
    if merged {
        sheet.cols.insert(2, LineInfo { size: Some(100.0), ..LineInfo::default() });
        sheet.merges.push(RangeRef::new(CellRef::new(4, 1), CellRef::new(4, 2)));
    }
    sheet.rows.insert(4, LineInfo { size: Some(44.0), custom: custom_height, ..LineInfo::default() });
    sheet.set_cell(CellRef::new(4, 1), Cell { value: Value::text("i".repeat(15)), style, ..Cell::default() });
    sheet.set_cell(CellRef::new(4, 3), Cell { value: Value::Number(0.0), ..Cell::default() });
    let mut session = Session::new();
    session.add_document(DocState::new(workbook, None, "Row height fixture".into()));
    session
}

fn make_app(session: Session) -> (SheetApp, Context) {
    let mut app = SheetApp::new(session, Default::default());
    let context = Context::default();
    for _ in 0..12 {
        let mut output = context.run_ui(RawInput::default(), |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        output.textures_delta.clear();
    }
    (app, context)
}

fn row_height(app: &SheetApp) -> f32 {
    app.session.active().expect("document exists").wb.active().expect("sheet exists").row_height(4)
}

fn edit_cell(app: &mut SheetApp, cell: &str, input: &str) {
    app.run("selection.set", json!({"range": cell})).expect("selection changes");
    app.begin_edit(Some(input.to_owned()), false);
    assert!(app.commit_edit(0, 0, false, false), "cell edit commits");
}

#[test]
fn open_autofit_short_neighbor_edit_and_undo_redo_keep_one_row_height_history() {
    let mut workbook = fixture(false, false).active().expect("document exists").wb.as_ref().clone();
    let bytes = gridcraft_engine::io::save_bytes(&workbook, "fixture.xlsx").expect("fixture saves");
    workbook = gridcraft_engine::io::open_bytes("fixture.xlsx", &bytes).expect("fixture opens").0;
    let mut session = Session::new();
    session.add_document(DocState::new(workbook, None, "Opened fixture".into()));
    let (mut app, context) = make_app(session);

    let style = app
        .session
        .active()
        .expect("document exists")
        .wb
        .styles
        .get(app.session.active().expect("document exists").wb.active().expect("sheet exists").style_id(CellRef::new(4, 1)));
    let sample = "i".repeat(15);
    let family = theme::cell_family(&context, &style.font.name, style.font.bold, style.font.italic);
    let active_width =
        context.fonts_mut(|fonts| fonts.layout_no_wrap(sample.clone(), FontId::new(style.font.size * 96.0 / 72.0, family), Color32::BLACK).size().x);
    let fallback_width = gridcraft_engine::cmd::format::approx_text_width(&sample, style.font.size, style.font.bold);
    assert!(
        active_width <= 105.0 && fallback_width > 105.0,
        "fixture must expose fallback overestimation: active={active_width}, fallback={fallback_width}"
    );

    app.run("home.autofitRowHeight", json!({"rows": "5:5"})).expect("AutoFit runs in active font context");
    let fitted = row_height(&app);
    assert!(fitted < 77.0, "active font shaping fits the short text in one line: {fitted}");
    let history_before_edit = app.session.active().expect("document exists").undo.len();

    edit_cell(&mut app, "D5", "1");
    let after_short_edit = row_height(&app);
    assert_eq!(after_short_edit, fitted, "editing a short neighbor leaves the row height stable");
    assert_eq!(app.session.active().expect("document exists").undo.len(), history_before_edit + 1, "the edit and row measurement are one undo step",);

    app.run("edit.undo", json!({})).expect("undo runs");
    assert_eq!(row_height(&app), fitted);
    app.run("edit.redo", json!({})).expect("redo runs");
    assert_eq!(row_height(&app), fitted);

    let bytes = gridcraft_engine::io::save_bytes(&app.session.active().expect("document exists").wb, "roundtrip.xlsx").expect("save succeeds");
    let (reopened, _) = gridcraft_engine::io::open_bytes("roundtrip.xlsx", &bytes).expect("reopen succeeds");
    assert_eq!(reopened.sheet(0).expect("sheet exists").row_height(4), fitted);
    assert!(!reopened.sheet(0).expect("sheet exists").rows.get(&4).is_some_and(|row| row.custom));
}

#[test]
fn wrapped_text_growth_shrink_newlines_and_merged_cells_measure_only_the_affected_row() {
    let (mut app, _) = make_app(fixture(false, false));
    app.run("home.autofitRowHeight", json!({"rows": "5:5"})).expect("AutoFit runs");
    let one_line = row_height(&app);

    edit_cell(&mut app, "B5", &"i".repeat(60));
    let longer = row_height(&app);
    assert!(longer > one_line, "longer wrapped content grows the row: {longer} > {one_line}");

    edit_cell(&mut app, "B5", "i");
    assert_eq!(row_height(&app), one_line, "shorter wrapped content shrinks back to one line");

    edit_cell(&mut app, "B5", "first\nsecond");
    assert!(row_height(&app) > one_line, "explicit newlines add a line");

    let (mut merged, _) = make_app(fixture(false, true));
    merged.run("home.autofitRowHeight", json!({"rows": "5:5"})).expect("merged AutoFit runs");
    let merged_one_line = row_height(&merged);
    edit_cell(&mut merged, "B5", &"i".repeat(25));
    assert_eq!(row_height(&merged), merged_one_line, "merged column width participates in wrapping");
}

#[test]
fn custom_height_survives_edits_and_font_size_changes_recompute_automatic_height() {
    let (mut custom, _) = make_app(fixture(true, false));
    let fixed = row_height(&custom);
    edit_cell(&mut custom, "B5", &"i".repeat(60));
    assert_eq!(row_height(&custom), fixed);
    assert!(custom.session.active().expect("document exists").wb.active().expect("sheet exists").rows.get(&4).is_some_and(|row| row.custom));

    let (mut automatic, _) = make_app(fixture(false, false));
    automatic.run("home.autofitRowHeight", json!({"rows": "5:5"})).expect("AutoFit runs");
    let small = row_height(&automatic);
    automatic.run("home.fontSize", json!({"range": "B5", "size": 36.0})).expect("font size changes");
    assert!(row_height(&automatic) > small, "larger font increases the automatic height");
}
