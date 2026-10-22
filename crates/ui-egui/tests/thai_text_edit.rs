use egui::{Event, ImeEvent, Key, Modifiers};
use egui_kittest::Harness;
use gridcraft_engine::{Session, core::CellRef, display::cell_text};
use gridcraft_ui_egui::{SheetApp, fonts, theme};
use serde_json::json;
use skrifa::MetadataProvider;

fn harness() -> Harness<'static, SheetApp> {
    let mut session = Session::new();
    session.new_workbook();
    let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let context = ui.ctx().clone();
            app.logic(&context);
            app.ui(ui);
        },
        SheetApp::new(session, Default::default()),
    );
    harness.run_steps(4);
    harness
}

fn key(harness: &mut Harness<'static, SheetApp>, key: Key, modifiers: Modifiers) {
    harness.input_mut().events.extend([
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
        Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
    ]);
    harness.run_steps(2);
}

fn click_caret(harness: &mut Harness<'static, SheetApp>, index: usize) {
    let app = harness.state();
    let doc = app.session.active().expect("workbook");
    let sheet = doc.wb.active().expect("active sheet");
    let edit = app.editor.as_ref().expect("editor");
    let style = doc.wb.styles.get(sheet.style_id(edit.cell));
    let family = theme::cell_family(&harness.ctx, &style.font.name, style.font.bold, style.font.italic);
    let font = egui::FontId::new(style.font.size * 96.0 / 72.0, family);
    let galley = harness.ctx.fonts_mut(|fonts| fonts.layout_no_wrap(edit.text.clone(), font, egui::Color32::BLACK));
    let caret = galley.pos_from_cursor(egui::text::CCursor::new(index));
    let cell = harness.state().grid.geo.as_ref().expect("grid geometry").cell_rect(sheet, edit.cell);
    let position = egui::pos2(cell.left() + 2.0 + caret.center().x, cell.center().y);
    harness.input_mut().events.extend([
        Event::PointerMoved(position),
        Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
    ]);
    harness.run_steps(2);
    harness.input_mut().events.push(Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(2);
}

fn face_has_char(face: &fonts::Face, chr: char) -> bool {
    std::fs::read(&face.path).is_ok_and(|bytes| skrifa::FontRef::from_index(&bytes, face.index).is_ok_and(|font| font.charmap().map(chr).is_some()))
}

#[test]
fn cell_editor_click_arrows_backspace_delete_and_selection_use_source_scalars() {
    let mut harness = harness();
    harness.state_mut().run("cell.set", json!({"cell":"A1", "input":"น้ำ"})).expect("set Thai cell");
    harness.state_mut().begin_edit(None, false);
    harness.run_steps(3);

    click_caret(&mut harness, 3);
    assert_eq!(harness.state().editor.as_ref().map(|edit| edit.caret), Some(3), "click maps to the source end of the shaped Thai cluster");

    key(&mut harness, Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(harness.state().editor.as_ref().map(|edit| edit.caret), Some(2));
    key(&mut harness, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(harness.state().editor.as_ref().map(|edit| edit.caret), Some(3));
    key(&mut harness, Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(harness.state().editor.as_ref().map(|edit| edit.caret), Some(2));
    key(&mut harness, Key::Backspace, Modifiers::NONE);
    assert_eq!(
        harness.state().editor.as_ref().map(|edit| edit.text.as_str()),
        Some("\u{0E19}\u{0E33}"),
        "Backspace removes one Unicode scalar (the tone mark) safely"
    );
    key(&mut harness, Key::Delete, Modifiers::NONE);
    assert_eq!(harness.state().editor.as_ref().map(|edit| edit.text.as_str()), Some("\u{0E19}"), "Delete removes the next Unicode scalar safely");

    harness.state_mut().cancel_edit();
    harness.state_mut().begin_edit(None, false);
    harness.run_steps(3);
    key(&mut harness, Key::ArrowLeft, Modifiers::SHIFT);
    let state = egui::TextEdit::load_state(&harness.ctx, egui::Id::new("gridcraft.cell_editor")).expect("text edit state");
    let font = egui::FontId::new(11.0 * 96.0 / 72.0, theme::cell_family(&harness.ctx, "Calibri", false, false));
    let galley = harness.ctx.fonts_mut(|fonts| fonts.layout_no_wrap("น้ำ".into(), font, egui::Color32::BLACK));
    let range = state.cursor.range(&galley).expect("select-all cursor range").as_sorted_char_range();
    assert_eq!((range.start.0, range.end.0), (2, 3), "Shift+Left selects one original Unicode scalar");

    harness.input_mut().events.push(Event::Copy);
    harness.run_steps(1);
    let copied = harness.output().platform_output.commands.iter().rev().find_map(|command| match command {
        egui::OutputCommand::CopyText(value) => Some(value.clone()),
        _ => None,
    });
    assert_eq!(copied.as_deref(), Some("ำ"), "copy preserves the selected original Unicode scalar");
    harness.input_mut().events.push(Event::Paste(copied.expect("copy command")));
    harness.run_steps(2);
    assert_eq!(harness.state().editor.as_ref().map(|edit| edit.text.as_str()), Some("น้ำ"), "paste restores the same source text");
}

#[test]
fn formula_bar_ime_preedit_commit_keeps_thai_source_and_reports_candidate_rect() {
    let mut harness = harness();
    harness.state_mut().begin_edit(Some("ภาษา".into()), true);
    harness.run_steps(3);
    harness.input_mut().events.push(Event::Ime(ImeEvent::Preedit { text: "ไทย".into(), active_range_chars: Some(0..3) }));
    harness.run_steps(2);
    let ime = harness.output().platform_output.ime.as_ref().expect("active IME composition output");
    assert!(ime.rect.is_finite(), "candidate rectangle must be finite: {:?}", ime.rect);
    assert!(harness.ctx.content_rect().contains(ime.rect.center()), "simulated candidate anchor should stay in the app content area: {:?}", ime.rect);

    harness.input_mut().events.push(Event::Ime(ImeEvent::Commit("ไทย".into())));
    harness.run_steps(2);
    assert_eq!(harness.state().editor.as_ref().map(|edit| edit.text.as_str()), Some("ภาษาไทย"));
    assert!(harness.output().platform_output.ime.is_some(), "TextEdit reports its cursor area to the platform while editing");
}

#[test]
fn autofit_uses_the_same_shaper_for_thai_column_width_and_wrapped_row_height() {
    let selected_family = fonts::catalog()
        .iter()
        .find(|face| !face.italic && (face.weight - 400.0).abs() < 1.0 && !face_has_char(face, '\u{0E20}'))
        .map(|face| face.family.clone());
    let thai_fallback_installed = fonts::catalog().iter().any(|face| face_has_char(face, '\u{0E20}'));
    let (Some(selected_family), true) = (selected_family, thai_fallback_installed) else {
        return; // This host has no installed selected face that needs Thai fallback.
    };

    let mut harness = harness();
    harness
        .state_mut()
        .run("cell.set", json!({"cell":"A1", "input":"\u{0E22}\u{0E2D}\u{0E14}\u{0E23}\u{0E27}\u{0E21} 1,234.50 \u{0E1A}\u{0E32}\u{0E17}"}))
        .expect("set Thai cell");
    harness
        .state_mut()
        .run("home.formatCells", json!({"range":"A1", "style":{"font":{"name":selected_family,"size":11.0}, "align":{"wrap":true}}}))
        .expect("set selected font and wrap");
    harness.run_steps(8); // Let workbook-font roles be rebound after the style change.

    let context = harness.ctx.clone();
    let doc = harness.state().session.active().expect("workbook");
    let sheet = doc.wb.active().expect("active sheet");
    let cell = CellRef::new(0, 0);
    let style = doc.wb.styles.get(sheet.style_id(cell));
    let source = cell_text(&doc.wb, sheet, cell);
    let font_id =
        egui::FontId::new(style.font.size * 96.0 / 72.0, theme::cell_family(&context, &style.font.name, style.font.bold, style.font.italic));
    let expected_column = context.fonts_mut(|fonts| fonts.layout_no_wrap(source.clone(), font_id.clone(), egui::Color32::BLACK).size().x) + 6.0;

    harness.state_mut().run("home.autofitColumnWidth", json!({"cols":"A:A"})).expect("autofit Thai column");
    let actual_column = harness.state().session.active().expect("workbook").wb.active().expect("sheet").col_width(0);
    assert_eq!(
        actual_column,
        expected_column.ceil(),
        "column width must be the pixel-rounded rendered no-wrap Galley: {actual_column} vs {expected_column}"
    );

    harness.state_mut().run("home.columnWidth", json!({"cols":"A:A", "width":42.0})).expect("set narrow column");
    harness.state_mut().run("home.rowHeight", json!({"rows":"1:1", "height":18.0})).expect("set short custom row");
    let doc = harness.state().session.active().expect("workbook");
    let sheet = doc.wb.active().expect("active sheet");
    let style = doc.wb.styles.get(sheet.style_id(cell));
    let wrap_width = (sheet.col_width(0) - 5.0 - style.align.indent as f32 * 9.0 - sheet.filter_button_space(cell)).max(4.0);
    let expected_row =
        context.fonts_mut(|fonts| fonts.layout_job(egui::text::LayoutJob::simple(source, font_id, egui::Color32::BLACK, wrap_width)).size().y + 5.0);
    assert!(expected_row > sheet.default_row_height, "sample must wrap beyond one row to test AutoFit");

    harness.state_mut().run("home.autofitRowHeight", json!({"rows":"1:1"})).expect("autofit wrapped Thai row");
    let sheet = harness.state().session.active().expect("workbook").wb.active().expect("sheet");
    let actual_row = sheet.row_height(0);
    assert!((actual_row - expected_row).abs() < 0.1, "row height must match rendered wrapped Galley: {actual_row} vs {expected_row}");
    assert!(!sheet.rows.get(&0).is_some_and(|row| row.custom), "AutoFit clears the custom row-height flag");
}

#[test]
fn autofit_wrapped_merged_cell_uses_the_same_content_width_as_painting() {
    let mut harness = harness();
    let source = "รหัสโครงการภาษาไทย";
    harness.state_mut().run("cell.set", json!({"cell":"A1", "input":source})).expect("set merged Thai cell");
    harness.state_mut().run("home.formatCells", json!({"range":"A1", "style":{"align":{"wrap":true}}})).expect("enable wrapping");
    harness.run_steps(4);

    let context = harness.ctx.clone();
    let doc = harness.state().session.active().expect("workbook");
    let sheet = doc.wb.active().expect("active sheet");
    let cell = CellRef::new(0, 0);
    let style = doc.wb.styles.get(sheet.style_id(cell));
    let font_id =
        egui::FontId::new(style.font.size * 96.0 / 72.0, theme::cell_family(&context, &style.font.name, style.font.bold, style.font.italic));
    let text_width = context.fonts_mut(|fonts| fonts.layout_no_wrap(source.to_owned(), font_id.clone(), egui::Color32::BLACK).size().x);
    let column_width = (text_width * 0.6).max(24.0);
    harness.state_mut().run("home.columnWidth", json!({"cols":"A:B", "width":column_width})).expect("size merge columns");
    harness.state_mut().run("home.mergeCells", json!({"range":"A1:B1"})).expect("merge the Thai cell");

    let doc = harness.state().session.active().expect("workbook");
    let sheet = doc.wb.active().expect("active sheet");
    let style = doc.wb.styles.get(sheet.style_id(cell));
    let merge = sheet.merge_at(cell).expect("merged anchor");
    let merged_width: f32 = (merge.start.col..=merge.end.col).map(|col| sheet.col_width(col)).sum();
    let filter_space = sheet.filter_button_space(cell);
    let wrap_width = (merged_width - 5.0 - style.align.indent as f32 * 9.0 - filter_space).max(4.0);
    assert!(wrap_width >= text_width, "the displayed merged area must fit the source line: {wrap_width} < {text_width}");
    assert!(column_width - 5.0 < text_width, "the anchor column alone must not fit the source line");
    let expected_galley =
        context.fonts_mut(|fonts| fonts.layout_job(egui::text::LayoutJob::simple(source.to_owned(), font_id, egui::Color32::BLACK, wrap_width)));
    assert_eq!(expected_galley.rows.len(), 1, "rendered merged cell must fit on one line");
    let expected_height = sheet.default_row_height.max(expected_galley.size().y + 5.0);

    harness.state_mut().run("home.rowHeight", json!({"rows":"1:1", "height":18.0})).expect("set a short row");
    harness.state_mut().run("home.autofitRowHeight", json!({"rows":"1:1"})).expect("autofit merged row");
    let sheet = harness.state().session.active().expect("workbook").wb.active().expect("sheet");
    let actual_height = sheet.row_height(0);
    assert!(
        (actual_height - expected_height).abs() < 0.1,
        "AutoFit must use the merged width used by painting: {actual_height} vs {expected_height}"
    );
}

#[test]
fn explicit_newlines_remain_visible_when_cell_wrap_is_disabled() {
    let mut harness = harness();
    let source = "ABCภาษาไทยXYZ\nยอดรวม 1,234.50 บาท";
    harness.state_mut().run("cell.set", json!({"cell":"A1", "input":source})).expect("set multiline cell");
    harness.state_mut().run("home.formatCells", json!({"range":"A1", "style":{"align":{"wrap":false}}})).expect("disable automatic wrapping");
    harness.state_mut().run("home.rowHeight", json!({"rows":"1:1", "height":18.0})).expect("make the row too short for both explicit lines");
    harness.state_mut().run("home.autofitRowHeight", json!({"rows":"1:1"})).expect("autofit explicit source lines");
    harness.run_steps(4);

    let grid_rect = harness.state().grid.rect.expect("grid rectangle");
    let displayed = harness.output().shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text)
            if grid_rect.contains(text.pos)
                && text.galley.job.text == source
                && text.galley.rows.len() == 2
                && shape.clip_rect.contains(text.pos) =>
        {
            Some((text, shape.clip_rect))
        }
        _ => None,
    });
    let (text, clip) = displayed.expect("the grid paints both explicit source lines");
    assert!(clip.bottom() - text.pos.y >= text.galley.size().y, "both lines fit inside the cell's vertical clip");
    let galley = &text.galley;
    let rows: Vec<String> = galley.rows.iter().map(|row| row.row.text()).collect();
    assert_eq!(rows, ["ABCภาษาไทยXYZ", "ยอดรวม 1,234.50 บาท"]);
    assert_eq!(rows.concat(), source.replace('\n', ""), "line boundaries must not insert or drop source characters");
    let row_height = harness.state().session.active().expect("workbook").wb.active().expect("sheet").row_height(0);
    assert!(row_height >= galley.size().y + 5.0, "AutoFit must reserve both explicit lines: {row_height} < {}", galley.size().y + 5.0);
    assert!(clip.bottom() - text.pos.y >= galley.size().y, "the post-AutoFit cell clip must contain both source lines");
}
