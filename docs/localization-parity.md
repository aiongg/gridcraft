# Localization parity

> **Last reviewed:** 2026-10-21 · **Last updated:** 2026-10-21 · **Change:** minor (PDF font lookup and shaping use bounded per-export caches) · **Target:** Microsoft Excel (Microsoft 365), Excel for Mac 16.113.4

Per-language status against Excel, which ships its UI in dozens of languages (Windows language
packs cover 100+), with localized function names, argument separators, number and date formats,
right-to-left sheets and full complex-script support. Part of
[target-app-parity.md](target-app-parity.md). **Localization: ~10% ready, 100–165 h** (estimated).

## Where we are (measured from source on `main`, after the 2026-10-10 merges)

- **A first catalog exists, for the ribbon only.** `crates/ui-egui/src/i18n.rs` (merged 10-10 in
  #31, #104, #82; German added 10-11) adds an interface-language preference that defaults to the
  system language (closes #20) and a View-tab switcher, with English-keyed lookup tables:
  Japanese 268 labels, Simplified Chinese, Korean, Russian, Brazilian Portuguese, German and Czech 266
  each. The tables are used by
  the ribbon, sheet tabs and widgets (`ribbon.rs`, `tabs.rs`, `widgets.rs`), the Format Cells
  dialog, every dialog title and the shared OK/Cancel buttons; the other dialogs, menus,
  context menus, task panes, status messages, command labels and function descriptions are still
  English literals. Untranslated labels fall back to English.
- **About 2,200 user-visible strings** in all (heuristic source scan: ~1,700 UI and command
  strings plus ~500 function descriptions), so each shipped language covers **~12%** (measured
  count / estimated total).
- **en-US only formulas and formats.** Function names, `,` argument separator, `.` decimal
  separator, en-US number and date formats (`crates/numfmt` says "en-US only"). Users ask for
  localized function names and `;` separators (#45). German with localized numbers and formulas
  is in PR #163 (open).
- **Script support.** System-font fallback for CJK, Thai and Hebrew landed in #38 (closes #32;
  #142 is still open pending confirmation). Installed workbook fonts keep the selected face first
  and inherit the system-script fallback chain for regular, bold, italic and bold-italic roles; a
  render check on Windows confirmed Thai glyphs through Leelawadee UI. The egui text shaper uses
  ICU4X Thai dictionary boundaries when words fit, and falls back inside overlong words only at
  shaped cluster boundaries. It retains the original Unicode text for cursor and selection
  indexing and uses the same Galley metrics and merged-cell content width for painting and
  UI-driven AutoFit. Editor arrows and deletion operate on Unicode scalar values, so removing a
  combining mark can leave a different but valid source sequence; this is not full grapheme-aware
  editing.
  Thai visual layout was reviewed in an offscreen render, but native IME candidate-window placement
  still needs platform testing. Arabic and Devanagari don't join or reorder, and there is no
  bidirectional layout. **Sheet Right-to-Left**
  (`view.rightToLeft`, Page Layout › Sheet Options, XLSX `rightToLeft`) mirrors the grid (#146):
  column A and the row headers on the right, the vertical scrollbar on the left; hit-testing,
  arrow keys, editor, popups and floating objects follow; text stays left-to-right inside its box
  (no bidi), and the sheet tabs, horizontal scrollbar and printing/PDF are not mirrored yet. IME input goes
  through egui/winit and is unverified for Chinese, Japanese and Korean.
- **PDF export** now embeds licensed TrueType fonts and shapes Thai text through the shared engine PDF pipeline. The Web/WASM caller must supply fonts at build time (`CRAFT_FONTS_DIR`) or per command (`pdfFonts`); native printer submission and other complex scripts remain unverified.

## Languages

UI strings translated are measured counts against the ~2,200 estimate. Estimates to `full`
include the rest of the catalog work below (10–20 h, counted once), localized function names,
separators, number/date formats, machine translation with agent review, and script work.

| Language | Code | UI strings translated | Dialogs / tooltips / help | Script support | Native review | Status | Estimate to full |
|---|---|---|---|---|---|---|---|
| English | en | ~2,200 / ~2,200 (100%, source) | yes | Latin: yes | yes | full | — |
| Simplified Chinese (Mandarin) | zh-Hans | 266 (~12%) | no | CJK system-font fallback (#38); IME unverified; PDF can't export | no | menus only | 8–13 h |
| Spanish | es | 0 (0%) | no | Latin: yes | no | none | 6–10 h |
| Hindi | hi | 0 (0%) | no | Devanagari: **no shaping** | no | none | 12–18 h |
| Arabic | ar | 0 (0%) | no | **no shaping, no RTL UI**; Sheet Right-to-Left mirrors the grid (#146) | no | none | 25–35 h |
| French | fr | 0 (0%) | no | Latin: yes | no | none | 6–10 h |
| Portuguese | pt-BR | 923 (~42%) | dialogs and tooltips (dialogs take effect with #669) | Latin: yes | no | menus, File view, dialogs | 2–4 h |
| Indonesian | id | 0 (0%) | no | Latin: yes | no | none | 5–8 h |
| Japanese | ja | 268 (~12%) | no | CJK system-font fallback; IME unverified; no furigana display | no | menus only | 8–12 h |
| German | de | 266 (~12%) | no | Latin: yes | no | menus only | 4–8 h |
| Korean | ko | 266 (~12%) | no | Hangul via the CJK fallback; IME unverified | no | menus only | 6–10 h |
| Vietnamese | vi | 0 (0%) | no | Latin with stacked diacritics: unverified | no | none | 6–10 h |

Other shipped languages: Russian (ru) and German (de), 266 labels each (~12%), menus only.

**Total:** catalog work 10–20 h + languages 92–144 h ≈ **100–165 h**. Parallelizes almost fully
once the catalog covers dialogs and commands. **Needs a human:** native-speaker review of every
language (none so far) and of localized function names.

## Infrastructure (do first)

1. Extend the catalog from the ribbon to every user-visible string (dialogs, menus, task panes,
   status and error messages, command labels, function descriptions), with a missing-key check
   in `cargo xtask ci`. Language switching and the system-language default already exist (#31).
2. Locale-aware formulas: localized function-name tables and separators at the formula-bar
   boundary only (files keep en-US invariant names), locale number and date formats, list
   separators in CSV.
3. Text shaping and print support: Thai PDF export/print now shapes cell, header/footer, shape and
   chart text with HarfRust, embeds permitted TrueType faces, maps visible glyph CIDs to source
   Unicode, and uses a logical ActualText span for extraction. Per-export font lookup, coverage
   and shaping data use bounded caches. It embeds full faces rather than subsets and rejects
   restricted/non-glyf fonts. Extend
   shaping beyond Thai to Arabic and Devanagari, add bidirectional runs and right-to-left sheet tabs,
   and implement font subsetting. Native export needs an embeddable system/craft font; Web/WASM
   needs `CRAFT_FONTS_DIR` at build time or licensed `pdfFonts` bytes in the command. Thai shaping
   still does not provide full grapheme-aware editing or native IME verification.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-21 | minor | Thai grid wrapping uses ICU4X dictionary boundaries with cluster-safe fallback for words wider than the cell |
| 2026-10-21 | minor | PDF face lookup, coverage checks and HarfRust shaping reuse bounded per-export caches; cached font additions invalidate prior lookup results |
| 2026-10-20 | minor | UI AutoFit publishes only complete column measurements within its 100,000-cell shaping budget; incomplete columns use engine fallback without discarding completed columns |
| 2026-10-20 | minor | Thai PDF/Print uses HarfRust glyph positions, permitted embedded TrueType faces and Unicode extraction mappings; native printer submission was not tested |
| 2026-10-20 | minor | epaint HarfRust-shaped Thai clusters stay intact across wraps; cursor mapping preserves source scalars, UI AutoFit uses the rendered Galley, and native IME positioning remains unverified |
| 2026-10-19 | minor | Installed workbook font roles keep the chosen face first and inherit the system-script fallback chain; Thai glyph rendering was verified with Leelawadee UI before the cluster-shaping work below |
| 2026-10-12 | minor | Format Cells looks up its tabs, labels, options and note through `Language::tr` (Fill tab keyed `Format Cells\|Fill`); dialog titles and OK/Cancel too. Labels already in a table (Number, Currency, Fill…) show translated; the rest fall back to English until the tables add them |
| 2026-10-12 | minor | Brazilian Portuguese catalog (`i18n/pt.rs`) grows from 377 to 923 entries: every key the other catalogs translate, including the dialog strings #669 starts looking up (~42%) |
| 2026-10-11 | minor | German ribbon catalog (`i18n/de.rs`, 266 labels, ~12%); `de` locale tags (de-DE, de-AT, de-CH…) resolve to German. PR #163 (localized German formulas/numbers) still open |
| 2026-10-11 | minor | Sheet Right-to-Left mirrors the grid (#146) |
| 2026-10-11 | minor | Czech (cs) ribbon table: 266 labels (~12%), menus only |
| 2026-10-10 | minor | Re-measured after #31, #104, #82, #38 merged: ribbon translated into ja, zh, ko, pt-BR and ru (266–268 labels, ~12%), system-language default, CJK/Thai/Hebrew font fallback; ~10% ready, 100–165 h |
| 2026-10-10 | major | Created: English only, no catalog, ~2,200 strings, script-support gaps, open language PRs |
