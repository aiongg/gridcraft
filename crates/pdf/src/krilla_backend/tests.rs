use super::*;
use craft_pdf::syntax::Pdf;
use craft_pdf::syntax::content::ops::TypedInstruction as I;
use craft_pdf::syntax::object::{Array, Dict, Name, Number, ObjectIdentifier, Stream, String as PdfString};
use std::sync::Arc;

/// Rounds to hundredths of a point, so both writers' number formatting compares equal.
fn r(v: f32) -> i64 {
    (v * 100.0).round() as i64
}

fn decode_text(bytes: &[u8]) -> String {
    match bytes {
        [0xFE, 0xFF, rest @ ..] => {
            let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
            String::from_utf16_lossy(&units)
        }
        _ => bytes.iter().map(|b| *b as char).collect(),
    }
}

#[derive(Debug, PartialEq)]
enum ReadTarget {
    Uri(String),
    /// Page index and the PDF-space (y-up) point.
    Page(usize, i64, i64),
}

/// A painted path: which paints, their colours, stroke width and dash, and its bounding box in
/// PDF space (control points included).
#[derive(Debug, PartialEq)]
struct Painted {
    fill: Option<[u8; 3]>,
    stroke: Option<([u8; 3], i64, Vec<i64>)>,
    bbox: [i64; 4],
}

#[derive(Debug, PartialEq)]
struct Placed {
    bbox: [i64; 4],
    pixels: (u32, u32),
    soft_mask: bool,
}

#[derive(Debug, PartialEq)]
struct ReadPage {
    size: [i64; 2],
    links: Vec<(Vec<i64>, ReadTarget)>,
    paths: Vec<Painted>,
    images: Vec<Placed>,
}

#[derive(Debug, PartialEq)]
struct ReadDoc {
    info: [Option<String>; 4],
    pages: Vec<ReadPage>,
}

#[derive(Clone)]
struct Gs {
    ctm: [f32; 6],
    fill: [u8; 3],
    stroke: [u8; 3],
    width: f32,
    dash: Vec<f32>,
}

fn mul(m: [f32; 6], n: [f32; 6]) -> [f32; 6] {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

fn apply(m: [f32; 6], x: f32, y: f32) -> [f32; 2] {
    [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]
}

fn bbox(pts: &[[f32; 2]]) -> [i64; 4] {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in pts {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    [r(x0), r(y0), r(x1), r(y1)]
}

fn rgb(c: [Number; 3]) -> [u8; 3] {
    c.map(|v| (v.as_f32() * 255.0).round() as u8)
}

/// Reads back what both writers must agree on: Info, page sizes, links, painted paths and
/// image placement.
fn read(bytes: &[u8]) -> ReadDoc {
    let pdf = Pdf::new(Arc::new(bytes.to_vec())).unwrap();
    let m = pdf.metadata();
    let info = [&m.title, &m.author, &m.creator, &m.producer].map(|v| v.as_deref().map(decode_text));
    let ids: Vec<Option<ObjectIdentifier>> = pdf.pages().iter().map(|p| p.raw().obj_id()).collect();
    let mut pages = Vec::new();
    for page in pdf.pages().iter() {
        let mb = page.media_box();
        let size = [r((mb.x1 - mb.x0) as f32), r((mb.y1 - mb.y0) as f32)];
        let mut links = Vec::new();
        if let Some(annots) = page.raw().get::<Array<'_>>(b"Annots") {
            for a in annots.iter::<Dict<'_>>() {
                let rect: Vec<i64> = a.get::<[f32; 4]>(b"Rect").unwrap().map(r).to_vec();
                let action = a.get::<Dict<'_>>(b"A");
                let dest = a.get::<Array<'_>>(b"Dest").or_else(|| action.as_ref().and_then(|d| d.get::<Array<'_>>(b"D")));
                let target = if let Some(uri) = action.as_ref().and_then(|d| d.get::<PdfString<'_>>(b"URI")) {
                    ReadTarget::Uri(decode_text(uri.as_bytes()))
                } else {
                    let mut it = dest.unwrap().flex_iter();
                    let target = it.next::<Dict<'_>>().unwrap().obj_id();
                    assert_eq!(it.next::<Name<'_>>().unwrap().as_str(), "XYZ");
                    let (x, y) = (it.next::<f32>().unwrap(), it.next::<f32>().unwrap());
                    ReadTarget::Page(ids.iter().position(|id| *id == target).unwrap(), r(x), r(y))
                };
                links.push((rect, target));
            }
        }
        let mut gs = Gs { ctm: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], fill: [0; 3], stroke: [0; 3], width: 1.0, dash: Vec::new() };
        let mut stack = Vec::new();
        let mut path: Vec<[f32; 2]> = Vec::new();
        let (mut paths, mut images) = (Vec::new(), Vec::new());
        let mut ops = page.typed_operations();
        while let Some(op) = ops.next() {
            let mut paint = |fill: bool, stroke: bool, path: &mut Vec<[f32; 2]>, gs: &Gs| {
                paths.push(Painted {
                    fill: fill.then_some(gs.fill),
                    stroke: stroke.then(|| (gs.stroke, r(gs.width), gs.dash.iter().map(|v| r(*v)).collect())),
                    bbox: bbox(path),
                });
                path.clear();
            };
            match op {
                I::SaveState(_) => stack.push(gs.clone()),
                I::RestoreState(_) => gs = stack.pop().unwrap(),
                I::Transform(t) => gs.ctm = mul([t.0, t.1, t.2, t.3, t.4, t.5].map(|n| n.as_f32()), gs.ctm),
                I::LineWidth(w) => gs.width = w.0.as_f32(),
                I::DashPattern(d) => gs.dash = d.0.iter::<f32>().collect(),
                I::NonStrokeColorDeviceRgb(c) => gs.fill = rgb([c.0, c.1, c.2]),
                I::StrokeColorDeviceRgb(c) => gs.stroke = rgb([c.0, c.1, c.2]),
                I::MoveTo(p) => path.push(apply(gs.ctm, p.0.as_f32(), p.1.as_f32())),
                I::LineTo(p) => path.push(apply(gs.ctm, p.0.as_f32(), p.1.as_f32())),
                I::CubicTo(c) => {
                    for (x, y) in [(c.0, c.1), (c.2, c.3), (c.4, c.5)] {
                        path.push(apply(gs.ctm, x.as_f32(), y.as_f32()));
                    }
                }
                I::RectPath(re) => {
                    let [x, y, w, h] = [re.0, re.1, re.2, re.3].map(|n| n.as_f32());
                    path.extend([apply(gs.ctm, x, y), apply(gs.ctm, x + w, y + h)]);
                }
                I::FillPathNonZero(_) | I::FillPathEvenOdd(_) => paint(true, false, &mut path, &gs),
                I::StrokePath(_) | I::CloseAndStrokePath(_) => paint(false, true, &mut path, &gs),
                I::FillAndStrokeNonZero(_) | I::CloseFillAndStrokeNonZero(_) => paint(true, true, &mut path, &gs),
                I::EndPath(_) => path.clear(),
                I::XObject(x) => {
                    let image: Stream<'_> = page.resources().get_x_object(x.0).unwrap();
                    let d = image.dict();
                    assert_eq!(d.get::<Name<'_>>(b"Subtype").unwrap().as_str(), "Image");
                    let corners = [apply(gs.ctm, 0.0, 0.0), apply(gs.ctm, 1.0, 1.0)];
                    images.push(Placed {
                        bbox: bbox(&corners),
                        pixels: (d.get::<u32>(b"Width").unwrap(), d.get::<u32>(b"Height").unwrap()),
                        soft_mask: d.contains_key(b"SMask"),
                    });
                }
                _ => {}
            }
        }
        pages.push(ReadPage { size, links, paths, images });
    }
    ReadDoc { info, pages }
}

fn gradient(pw: u32, ph: u32, alpha: bool) -> Vec<u8> {
    (0..pw * ph).flat_map(|i| [(i * 13) as u8, (i * 7) as u8, 90, if alpha && i % 3 == 0 { 100 } else { 255 }]).collect()
}

/// The same non-text scene for either writer; `$doc.add_page` yields the page.
macro_rules! scene {
    ($doc:expr, $add:ident) => {{
        $doc.info.title = "Q1 résumé ✓".into();
        $doc.info.author = "A. Author".into();
        let p = $add!($doc, 400.0, 300.0);
        p.fill_rect(10.0, 20.0, 100.0, 30.0, [255, 230, 0]);
        p.rect_stroke(10.0, 20.0, 100.0, 30.0, 0.5, [0, 0, 0]);
        p.stroke_line(0.0, 0.0, 400.0, 300.0, 1.25, [200, 0, 0]);
        p.stroke_line_dashed(5.0, 60.0, 205.0, 60.0, 1.0, [0, 0, 128], &[3.0, 2.0]);
        p.polyline(&[[20.0, 100.0], [60.0, 140.0], [100.0, 110.0]], 2.0, [0, 128, 0], &[4.0, 1.0, 1.0, 1.0]);
        p.polygon(&[[150.0, 100.0], [190.0, 100.0], [170.0, 140.0]], Some([0, 0, 255]), Some(([10, 10, 10], 1.0)));
        p.polygon(&[[200.0, 100.0], [240.0, 100.0], [220.0, 140.0]], None, Some(([40, 40, 40], 0.75)));
        p.circle(300.0, 200.0, 25.0, Some([0, 160, 80]), Some(([90, 0, 90], 1.5)));
        p.circle(330.0, 60.0, 10.0, Some([1, 2, 3]), None);
        p.save();
        p.clip_rect(0.0, 0.0, 50.0, 50.0);
        p.fill_rect(5.0, 5.0, 60.0, 60.0, [128, 128, 128]);
        p.restore();
        p.image_rgba(250.0, 20.0, 80.0, 40.0, &gradient(8, 4, true), 8, 4);
        p.image_rgba(20.0, 200.0, 30.0, 30.0, &gradient(3, 3, false), 3, 3);
        p.link_uri(10.0, 20.0, 100.0, 30.0, "https://example.org/a(b)?q=£");
        p.link_uri(-20.0, -20.0, 40.0, 40.0, "mailto:a@example.org");
        p.link_page(300.0, 250.0, 80.0, 40.0, 1, 35.0, 60.0);
        let q = $add!($doc, 200.0, 500.0);
        q.fill_rect(0.0, 480.0, 200.0, 20.0, [10, 20, 30]);
        q.image_rgba(50.0, 50.0, 100.0, 100.0, &gradient(4, 4, true), 4, 4);
        q.link_page(0.0, 0.0, 20.0, 20.0, 0, 0.0, 0.0);
        q.link_page(0.0, 0.0, 20.0, 20.0, 7, 0.0, 0.0);
    }};
}

macro_rules! add_old {
    ($doc:expr, $w:expr, $h:expr) => {
        $doc.add_page($w, $h)
    };
}
macro_rules! add_new {
    ($doc:expr, $w:expr, $h:expr) => {
        $doc.add_page($w, $h).unwrap()
    };
}

fn new_scene() -> Vec<u8> {
    let mut d = Document::new();
    scene!(d, add_new);
    d.finish().unwrap()
}

#[test]
fn matches_the_hand_written_writer() {
    let mut old = crate::PdfDoc::new();
    scene!(old, add_old);
    let old = read(&old.finish());
    let new = read(&new_scene());
    assert_eq!(old.info, new.info);
    assert_eq!(new.info[0].as_deref(), Some("Q1 résumé ✓"));
    assert_eq!(old.pages.len(), 2);
    for (o, n) in old.pages.iter().zip(&new.pages) {
        assert_eq!(o.size, n.size);
        assert_eq!(o.links, n.links);
        assert_eq!(o.images, n.images);
        assert_eq!(o.paths, n.paths);
    }
    assert_eq!(old.pages, new.pages);
    // The scene exercises every kind of output being compared.
    let first = &new.pages[0];
    assert_eq!((first.links.len(), first.images.len(), first.paths.len()), (3, 2, 10));
    assert!(first.images[0].soft_mask && !first.images[1].soft_mask);
    assert_eq!(new.pages[1].links.len(), 1);
}

#[test]
fn output_is_deterministic() {
    assert_eq!(new_scene(), new_scene());
}

#[test]
fn ops_are_recorded_in_drawing_order() {
    let mut d = Document::new();
    let p = d.add_page(100.0, 100.0).unwrap();
    p.save();
    p.clip_rect(0.0, 0.0, 10.0, f32::NAN);
    p.fill_rect(1.0, 2.0, 3.0, 4.0, [9, 9, 9]);
    p.restore();
    p.restore();
    p.stroke_line_dashed(0.0, 0.0, 5.0, 0.0, -1.0, [0, 0, 0], &[0.0, 0.0]);
    let line = Line { color: [0, 0, 0], width: 0.0, dash: None, round_join: false };
    assert_eq!(
        p.ops(),
        &[
            Op::Save,
            Op::ClipRect { x: 0.0, y: 0.0, w: 10.0, h: 0.0 },
            Op::Path { shape: Shape::Rect { x: 1.0, y: 2.0, w: 3.0, h: 4.0 }, fill: Some([9, 9, 9]), stroke: None },
            Op::Restore,
            Op::Path { shape: Shape::Polyline(vec![[0.0, 0.0], [5.0, 0.0]]), fill: None, stroke: Some(line) },
        ]
    );
}

#[test]
fn page_sizes_are_clamped() {
    let mut d = Document::new();
    for (w, h) in [(1.0e9, 1.0), (f32::NAN, f32::INFINITY), (-5.0, 600.0)] {
        d.add_page(w, h).unwrap();
    }
    let sizes: Vec<[i64; 2]> = read(&d.finish().unwrap()).pages.iter().map(|p| p.size).collect();
    assert_eq!(sizes, [[1_440_000, 300], [61_200, 79_200], [300, 60_000]]);
    let blank = read(&Document::new().finish().unwrap());
    assert_eq!(blank.pages.iter().map(|p| p.size).collect::<Vec<_>>(), [[61_200, 79_200]]);
}

#[test]
fn page_count_is_capped() {
    let mut d = Document::new();
    for _ in 0..MAX_PAGES {
        d.add_page(10.0, 10.0).unwrap();
    }
    assert!(matches!(d.add_page(10.0, 10.0), Err(PdfError::PageLimit)));
    assert_eq!(d.page_count(), MAX_PAGES);
}

#[test]
fn oversized_and_malformed_images_are_ignored() {
    let mut d = Document::new();
    let p = d.add_page(100.0, 100.0).unwrap();
    let px = [255u8; 16];
    p.image_rgba(0.0, 0.0, 10.0, 10.0, &[], u32::MAX, u32::MAX);
    // Over MAX_IMAGE_PIXELS with a buffer the size the dimensions claim is never copied: the
    // pixel bound is checked before the length.
    p.image_rgba(0.0, 0.0, 10.0, 10.0, &px, 40_000, 1_001);
    p.image_rgba(0.0, 0.0, 10.0, 10.0, &px[..15], 2, 2);
    p.image_rgba(0.0, 0.0, 10.0, 10.0, &px, 0, 4);
    for (w, h) in [(f32::NAN, 10.0), (10.0, f32::INFINITY), (-1.0, 10.0), (10.0, 0.0)] {
        p.image_rgba(0.0, 0.0, w, h, &px, 2, 2);
    }
    assert!(p.ops().is_empty());
    // A longer buffer is cut to the pixels the dimensions name.
    p.image_rgba(0.0, 0.0, 10.0, 10.0, &[7u8; 64], 2, 2);
    assert!(matches!(&p.ops()[0], Op::Image { rgba, .. } if rgba.len() == 16));
}

#[test]
fn links_are_filtered_and_capped() {
    let mut d = Document::new();
    let p = d.add_page(100.0, 100.0).unwrap();
    for uri in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "data:text/html,x",
        "https:",
        "no-scheme",
        "https://example.org/\nnext",
        format!("https://example.org/{}", "a".repeat(MAX_LINK_URI_BYTES)).as_str(),
    ] {
        p.link_uri(0.0, 0.0, 10.0, 10.0, uri);
    }
    for (x, y, w, h) in [
        (f32::NAN, 0.0, 10.0, 10.0),
        (0.0, 0.0, -1.0, 10.0),
        (0.0, 0.0, f32::INFINITY, 1.0),
        (200.0, 0.0, 10.0, 10.0),
        (f32::MAX, 0.0, f32::MAX, 1.0),
    ] {
        p.link_uri(x, y, w, h, "https://example.org");
    }
    p.link_page(0.0, 0.0, 10.0, 10.0, 0, f32::NAN, 0.0);
    assert!(p.links().is_empty());
    p.link_uri(90.0, 90.0, 20.0, 20.0, "HTTPS://example.org/é");
    assert_eq!(p.links(), &[Link { rect: [90.0, 90.0, 100.0, 100.0], target: LinkTarget::Uri("HTTPS://example.org/%C3%A9".into()) }]);
    for _ in 0..MAX_PAGE_LINKS + 10 {
        p.link_page(0.0, 0.0, 10.0, 10.0, 5, 0.0, 0.0);
    }
    assert_eq!(p.links().len(), MAX_PAGE_LINKS);
    // Links to a page that doesn't exist are dropped instead of failing the export.
    let doc = read(&d.finish().unwrap());
    assert_eq!(doc.pages[0].links.len(), 1);
}

#[test]
fn too_much_page_content_is_an_error() {
    let mut d = Document::new();
    d.add_page(100.0, 100.0).unwrap().fill_rect(0.0, 0.0, 1.0, 1.0, [0, 0, 0]);
    let p = d.add_page(100.0, 100.0).unwrap();
    let pts = vec![[1.0, 2.0]; 100_000];
    for _ in 0..30 {
        p.polyline(&pts, 1.0, [0, 0, 0], &[]);
    }
    assert!(p.ops().is_empty(), "the dropped page holds no content");
    assert!(matches!(d.finish(), Err(PdfError::ContentLimit { page: 1 })));
}

#[test]
fn too_deep_nesting_is_an_error() {
    let mut d = Document::new();
    let p = d.add_page(100.0, 100.0).unwrap();
    for _ in 0..MAX_NESTING / 2 {
        p.save();
        p.clip_rect(0.0, 0.0, 50.0, 50.0);
    }
    assert!(d.finish().is_ok(), "nesting up to the bound is written");
    let mut d = Document::new();
    let p = d.add_page(100.0, 100.0).unwrap();
    for _ in 0..=MAX_NESTING {
        p.save();
    }
    assert!(matches!(d.finish(), Err(PdfError::NestingLimit { page: 0 })));
}

#[test]
fn info_strings_are_capped() {
    let mut d = Document::new();
    d.info.title = "é".repeat(MAX_INFO_CHARS + 50);
    d.info.creator = "x".repeat(MAX_INFO_CHARS * 2);
    let doc = read(&d.finish().unwrap());
    assert_eq!(doc.info[0].as_ref().map(|t| t.chars().count()), Some(MAX_INFO_CHARS));
    assert_eq!(doc.info[1], None);
    assert_eq!(doc.info[2].as_ref().map(String::len), Some(MAX_INFO_CHARS));
    assert_eq!(doc.info[3].as_deref(), Some(PRODUCER));
}

#[test]
fn every_draw_method_survives_hostile_numbers() {
    let bad = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -7.0, 0.0, 1.0e30, -1.0e30, f32::MAX];
    let px = [200u8; 16];
    let mut d = Document::new();
    let p = d.add_page(300.0, 300.0).unwrap();
    for &a in &bad {
        for &b in &bad {
            p.save();
            p.clip_rect(a, b, a, b);
            p.fill_rect(a, b, b, a, [1, 2, 3]);
            p.rect_stroke(a, b, a, b, a, [1, 2, 3]);
            p.stroke_line(a, b, b, a, a, [1, 2, 3]);
            p.stroke_line_dashed(a, b, b, a, b, [1, 2, 3], &[a, b, 0.0]);
            p.polyline(&[[a, b], [b, a], [1.0, 1.0]], a, [1, 2, 3], &[b, a]);
            p.polygon(&[[a, b], [b, a], [1.0, 1.0]], Some([1, 2, 3]), Some(([4, 5, 6], a)));
            p.circle(a, b, b, Some([1, 2, 3]), Some(([4, 5, 6], a)));
            p.image_rgba(a, b, b, a, &px, 2, 2);
            p.link_uri(a, b, b, a, "https://example.org");
            p.link_page(a, b, b, a, 0, a, b);
            if a.is_nan() {
                p.restore();
            }
        }
        p.clip_rect(a, a, a, a);
    }
    p.stroke_line_dashed(0.0, 0.0, 1.0, 1.0, 1.0, [0, 0, 0], &[0.0; 100]);
    let finite_ops = p.ops().iter().all(|op| match op {
        Op::ClipRect { x, y, w, h } | Op::Image { x, y, w, h, .. } => finite(&[*x, *y, *w, *h]),
        Op::Path { shape: Shape::Rect { x, y, w, h }, .. } => finite(&[*x, *y, *w, *h]),
        Op::Path { shape: Shape::Circle { cx, cy, r }, .. } => finite(&[*cx, *cy, *r]),
        Op::Path { shape: Shape::Polyline(p) | Shape::Polygon(p), .. } => p.iter().all(|p| finite(p)),
        Op::Save | Op::Restore => true,
    });
    assert!(finite_ops);
    read(&d.finish().unwrap());
}
