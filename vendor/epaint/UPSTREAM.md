# Vendored epaint

This directory contains epaint 0.36.2 from the published crate at
https://crates.io/crates/epaint/0.36.2. Its original source and crate metadata are retained here;
the package is licensed under either MIT or Apache-2.0, with the corresponding upstream license
texts in `LICENSE-MIT` and `LICENSE-APACHE`.

GridCraft patches the text layout implementation because egui 0.36.2 uses HarfRust to shape text
clusters but its row wrapping and cursor code assumes a one-to-one relationship between shaped
glyphs and source characters. Thai clusters can emit a different number of glyphs than source
scalars. The patch records source ranges for shaped clusters, only offers line breaks at cluster
boundaries, preserves source text for rows, and maps cursor slots back to source Unicode scalars
without changing the shaped glyph mesh. See `src/text/text_layout.rs`,
`src/text/text_layout_types.rs`, and `src/shapes/text_shape.rs`.

The crate is selected through the root workspace's `[patch.crates-io]` entry. Revisit or remove this
vendor when upstream epaint provides equivalent cluster-safe wrapping and source-indexed cursor
mapping. No font files are included.
