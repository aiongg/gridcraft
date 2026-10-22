use egui::{Color32, FontFamily, FontId, text::LayoutJob};
use gridcraft_ui_egui::{fonts, i18n::Language, theme};
use skrifa::MetadataProvider;
use unicode_segmentation::UnicodeSegmentation as _;

const THAI_SAMPLES: &[&str] = &[
    "ภาษาไทย",
    "รหัสโครงการ",
    "ค่าน้ำมันรถยนต์",
    "ค่าโดยสารสาธารณะ",
    "น้ำ",
    "กุ้ง",
    "ผู้ใช้",
    "เก้าอี้",
    "ป ป ญ ฐ",
    "ยอดรวม 1,234.50 บาท",
    "Tetra Pak สวัสดี",
    "ABCภาษาไทยXYZ",
    "ABC ภาษาไทย XYZ",
    "ยอดรวม1,234.50บาท / () - + % ฿ € © °",
    "น้ำ กุ้ง ผู้ใช้ เก้าอี้ ปี่ ปู่ ญู ฐุ",
    "English ไทย English",
    "ภาษาไทย English ข้อความหลายบรรทัด\nยอดรวม 1,234.50 บาท",
    "ข้อความยาวภาษาไทย English ข้อความยาวภาษาไทย English ข้อความยาวภาษาไทย English ข้อความยาวภาษาไทย English ข้อความยาวภาษาไทย English ข้อความยาวภาษาไทย English ข้อความยาวภาษาไทย English ข้อความยาวภาษาไทย English ข้อความยาวภาษาไทย English ข้อความยาวภาษาไทย English ข้อความยาวภาษาไทย English ข้อความยาวภาษาไทย English",
];

fn has_char(face: &fonts::Face, chr: char) -> bool {
    std::fs::read(&face.path).is_ok_and(|bytes| skrifa::FontRef::from_index(&bytes, face.index).is_ok_and(|font| font.charmap().map(chr).is_some()))
}

fn thai_family_without_thai() -> Option<String> {
    fonts::catalog().iter().find(|face| !face.italic && (face.weight - 400.0).abs() < 1.0 && !has_char(face, 'ภ')).map(|face| face.family.clone())
}

fn assert_galley_matches_source(text: &str, galley: &egui::Galley) {
    let mut reconstructed = String::new();
    let mut source_char_offset = 0;
    let boundaries: std::collections::HashSet<usize> = text
        .grapheme_indices(true)
        .map(|(byte, _)| text.get(..byte).map_or(0, |prefix| prefix.chars().count()))
        .chain(std::iter::once(text.chars().count()))
        .collect();

    for placed in &galley.rows {
        let row_text = placed.row.text();
        assert_eq!(placed.row.glyphs.len(), row_text.chars().count(), "one cursor slot per source scalar: {row_text:?}");
        assert_eq!(placed.row.char_count_excluding_newline().0, row_text.chars().count());
        assert!(
            !placed.row.visuals.mesh.vertices.is_empty() || row_text.chars().all(char::is_whitespace),
            "non-whitespace row must have a rendered mesh: {row_text:?} in {text:?}"
        );
        source_char_offset += row_text.chars().count();
        reconstructed.push_str(&row_text);
        if placed.ends_with_newline {
            source_char_offset += 1;
            reconstructed.push('\n');
        } else {
            assert!(boundaries.contains(&source_char_offset), "wrap split a grapheme in {text:?} at {source_char_offset}");
        }
    }

    assert_eq!(reconstructed, text, "layout rows must retain the exact source Unicode");
    assert_eq!(source_char_offset, text.chars().count(), "row cursor map must cover every source scalar");
    assert_eq!(galley.end().index.0, text.chars().count(), "Galley cursor indices must map to source scalar indices");

    for index in 0..=text.chars().count() {
        let cursor = egui::text::CCursor::new(index);
        let caret = galley.pos_from_cursor(cursor);
        assert!(caret.is_finite(), "caret {index} must have finite display coordinates for {text:?}: {caret:?}");
        let hit = galley.cursor_from_pos(caret.center().to_vec2());
        assert!(hit.index.0 <= text.chars().count(), "click mapping must remain in source bounds");
    }
}

fn shaped_galley(ctx: &egui::Context, text: &str, family: FontFamily, width: f32) -> std::sync::Arc<egui::Galley> {
    let job = LayoutJob::simple(text.to_owned(), FontId::new(20.0, family), Color32::BLACK, width);
    ctx.fonts_mut(|fonts| fonts.layout_job(job))
}

#[test]
fn thai_words_wrap_at_dictionary_boundaries_when_each_word_fits() {
    // Built-in fonts keep this geometry regression independent of the host's font catalog.
    let context = egui::Context::default();
    let mut output = context.run_ui(egui::RawInput::default(), |_ui| {});
    output.textures_delta.clear();
    let font = FontId::monospace(20.0);
    let cases: &[(&str, &[&str])] = &[
        ("รหัสโครงการ", &["รหัส", "โครงการ"]),
        ("ค่าน้ำมันรถยนต์", &["ค่าน้ำมัน", "รถยนต์"]),
        ("ค่าโดยสารสาธารณะ", &["ค่า", "โดยสาร", "สาธารณะ"]),
        ("ABCภาษาไทยXYZ", &["ABC", "ภาษา", "ไทย", "XYZ"]),
    ];
    for (text, words) in cases {
        let widths = context.fonts_mut(|fonts| {
            words.iter().map(|word| fonts.layout_no_wrap((*word).to_owned(), font.clone(), Color32::BLACK).size().x).collect::<Vec<_>>()
        });
        let text_width = context.fonts_mut(|fonts| fonts.layout_no_wrap((*text).to_owned(), font.clone(), Color32::BLACK).size().x);
        let wrap_width = widths.into_iter().fold(0.0_f32, f32::max) + 0.5;
        assert!(wrap_width < text_width, "every word in {text:?} must fit, while the full source does not");

        let galley = shaped_galley(&context, text, FontFamily::Monospace, wrap_width);
        let rows: Vec<String> = galley.rows.iter().map(|row| row.row.text()).collect();
        let expected_rows: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
        assert_eq!(rows, expected_rows, "Thai dictionary boundaries for {text:?}");
        assert_eq!(rows.concat(), *text, "line wrapping must not alter source Unicode");
    }
}

#[test]
fn overlong_thai_words_fall_back_at_grapheme_boundaries() {
    let context = egui::Context::default();
    let mut output = context.run_ui(egui::RawInput::default(), |_ui| {});
    output.textures_delta.clear();
    let font = FontId::monospace(20.0);
    let text = "รหัสโครงการ";
    let narrow_width = context.fonts_mut(|fonts| fonts.layout_no_wrap("รหัส".to_owned(), font.clone(), Color32::BLACK).size().x * 0.6);
    let galley = shaped_galley(&context, text, FontFamily::Monospace, narrow_width);

    assert!(narrow_width < context.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), font, Color32::BLACK).size().x));
    assert!(galley.rows.len() > 2, "a word that cannot fit may wrap within the word");
    assert_galley_matches_source(text, &galley);
}

#[test]
fn thai_fallback_layout_maps_shaped_glyphs_to_source_and_keeps_graphemes_together() {
    let selected_family = thai_family_without_thai();
    let thai_font = fonts::catalog().iter().find(|face| has_char(face, 'ภ'));
    let (Some(selected_family), Some(_thai_font)) = (selected_family, thai_font) else {
        return; // This host has no Thai fallback or no Latin face that needs it.
    };

    let definitions = theme::font_definitions_with(Language::En, std::slice::from_ref(&selected_family));
    if !definitions.font_data.contains_key("sys-thai") {
        return; // No loadable Thai font is installed; the environment limitation is reported.
    }

    let context = egui::Context::default();
    context.set_fonts(definitions.clone());
    let mut output = context.run_ui(egui::RawInput::default(), |_ui| {});
    output.textures_delta.clear();

    let styles = [(false, false), (true, false), (false, true), (true, true)];
    for (bold, italic) in styles {
        let role = fonts::role_key(&selected_family, bold, italic);
        let role_family = FontFamily::Name(role.clone().into());
        let chain = definitions.families.get(&role_family).expect("workbook font role must be bound");
        assert!(chain.first().is_some_and(|key| key.starts_with(&format!("fnt-src:{selected_family}:"))));
        assert!(chain.iter().any(|key| key == "sys-thai"), "{role} needs Thai script fallback");
        assert!(chain.iter().all(|key| definitions.font_data.contains_key(key)), "{role} must only reference loaded font data");

        for text in THAI_SAMPLES {
            let width = if text == &"น้ำ" {
                14.0
            } else if text.contains("หลายบรรทัด") {
                68.0
            } else {
                92.0
            };
            let galley = shaped_galley(&context, text, role_family.clone(), width);
            assert_galley_matches_source(text, &galley);
            if text == &"น้ำ" {
                assert!(galley.rows.len() <= 2, "a Thai grapheme may overflow a narrow column, but must not be torn apart");
            }
        }
    }

    let mut thai_families = Vec::new();
    for face in fonts::catalog().iter().filter(|face| has_char(face, 'ภ')) {
        if !thai_families.iter().any(|family: &String| family == &face.family) {
            thai_families.push(face.family.clone());
            if thai_families.len() == 2 {
                break;
            }
        }
    }
    eprintln!("Installed Thai font families exercised: {}", thai_families.join(", "));
    for family in thai_families {
        let family_fonts = theme::font_definitions_with(Language::En, std::slice::from_ref(&family));
        let family_context = egui::Context::default();
        family_context.set_fonts(family_fonts);
        let mut output = family_context.run_ui(egui::RawInput::default(), |_ui| {});
        output.textures_delta.clear();
        let role = fonts::role_key(&family, false, false);
        let galley = shaped_galley(&family_context, "ภาษาไทย น้ำ กุ้ง ผู้ใช้ เก้าอี้", FontFamily::Name(role.into()), 68.0);
        assert_galley_matches_source("ภาษาไทย น้ำ กุ้ง ผู้ใช้ เก้าอี้", &galley);
    }
}
