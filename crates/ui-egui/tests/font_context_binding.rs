use egui::{Align2, Color32, Context, FontDefinitions, FontFamily, FontId, RawInput, pos2};
use gridcraft_ui_egui::{fonts, theme};

const FAMILY: &str = "GridCraft Fixture Font";
const SAMPLE: &str = "Fixture ABC";

fn definitions(with_fixture: bool) -> FontDefinitions {
    let mut definitions = FontDefinitions::default();
    let proportional = definitions.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    for role in [theme::UI, theme::UI_BOLD, theme::CELL, theme::CELL_BOLD, theme::CELL_ITALIC, theme::CELL_BOLD_ITALIC, theme::SERIF, theme::MONO] {
        definitions.families.insert(FontFamily::Name(role.into()), proportional.clone());
    }
    if with_fixture {
        for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
            let role = fonts::role_key(FAMILY, bold, italic);
            definitions.families.insert(FontFamily::Name(role.into()), proportional.clone());
        }
    }
    definitions
}

fn activate(context: &Context) {
    let mut output = context.run_ui(RawInput::default(), |_ui| {});
    output.textures_delta.clear();
}

fn select(context: &Context, name: &str, bold: bool, italic: bool) -> FontFamily {
    theme::cell_family(context, name, bold, italic)
}

fn measure(context: &Context, family: FontFamily) {
    context.fonts_mut(|fonts| {
        fonts.layout_no_wrap(SAMPLE.to_owned(), FontId::new(14.0, family), Color32::BLACK);
    });
}

fn draw(context: &Context, family: FontFamily) {
    let mut output = context.run_ui(RawInput::default(), |ui| {
        ui.painter().text(pos2(0.0, 0.0), Align2::LEFT_TOP, SAMPLE, FontId::new(14.0, family.clone()), Color32::BLACK);
    });
    output.textures_delta.clear();
}

#[test]
fn cell_font_roles_follow_the_context_being_measured_and_drawn() {
    let context_with_font = Context::default();
    context_with_font.set_fonts(definitions(true));
    activate(&context_with_font);

    let context_without_font = Context::default();
    context_without_font.set_fonts(definitions(false));
    activate(&context_without_font);

    for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
        let selected = select(&context_with_font, FAMILY, bold, italic);
        assert_eq!(selected, FontFamily::Name(fonts::role_key(FAMILY, bold, italic).into()));
        measure(&context_with_font, selected.clone());
        draw(&context_with_font, selected);

        let fallback = FontFamily::Name(
            match (bold, italic) {
                (true, true) => theme::CELL_BOLD_ITALIC,
                (true, false) => theme::CELL_BOLD,
                (false, true) => theme::CELL_ITALIC,
                (false, false) => theme::CELL,
            }
            .into(),
        );
        let selected_without_font = select(&context_without_font, FAMILY, bold, italic);
        assert_eq!(selected_without_font, fallback, "the second Context has no fixture family");
        measure(&context_without_font, selected_without_font.clone());
        draw(&context_without_font, selected_without_font);
    }
}

#[test]
fn pending_font_definitions_are_not_selected_before_the_next_pass_activates_them() {
    let context = Context::default();
    context.set_fonts(definitions(false));
    activate(&context);

    let new_definitions = definitions(true);
    context.set_fonts(new_definitions);

    let active_fallback = select(&context, FAMILY, false, false);
    assert_eq!(active_fallback, FontFamily::Name(theme::CELL.into()), "set_fonts takes effect at the next pass");
    measure(&context, active_fallback);

    activate(&context);
    let active_family = select(&context, FAMILY, false, false);
    assert_eq!(active_family, FontFamily::Name(fonts::role_key(FAMILY, false, false).into()));
    measure(&context, active_family.clone());
    draw(&context, active_family);
}
