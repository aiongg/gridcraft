//! The krilla-based PDF writer, through craft-pdf's guarded `write` API.
//!
//! [`Document`] and [`Page`] keep [`PdfDoc`](crate::PdfDoc)'s drawing API (top-left origin, y
//! grows down, points) apart from text. A page records its drawing as a list of [`Op`]s and
//! [`Document::finish`] renders every page through krilla, so text runs can later join the same
//! op list.
//!
//! Input bounds match the hand-written writer: page sizes are clamped, links are limited and
//! filtered to http, https and mailto, and images above [`MAX_IMAGE_PIXELS`] are ignored. Too many
//! pages, too much content on a page or too deep save/clip nesting is an error from
//! [`Document::add_page`] or [`Document::finish`] instead of silently missing output. The output
//! depends only on the input: no clock, no random IDs.

use craft_pdf::write::{self, Metadata, WriteError, Writer, backend};

use backend::action::LinkAction;
use backend::annotation::{Annotation, LinkAnnotation, Target};
use backend::color::rgb;
use backend::destination::XyzDestination;
use backend::geom::{Path, PathBuilder, Point, Rect, Size, Transform};
use backend::image::Image;
use backend::page::PageSettings;
use backend::paint::{Fill, FillRule, LineJoin, Stroke, StrokeDash};
use backend::surface::Surface;

use crate::{Info, MAX_IMAGE_PIXELS, MAX_INFO_CHARS, MAX_LINK_URI_BYTES, MAX_PAGE_CONTENT, MAX_PAGE_LINKS, Rgb, extent, finite, stroke_width};

/// Most pages in one document.
pub const MAX_PAGES: usize = 2000;
/// Most save levels plus clips in effect at once on a page.
pub const MAX_NESTING: usize = 4096;
/// Largest absolute coordinate written; larger finite values are clamped to it.
const MAX_COORD: f32 = 1.0e7;
/// The Info dictionary's Producer.
const PRODUCER: &str = "GridCraft PDF";

/// Why a document could not be written.
#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("the document has more than {MAX_PAGES} pages")]
    PageLimit,
    #[error("page {page} has more than {MAX_PAGE_CONTENT} bytes of content")]
    ContentLimit { page: usize },
    #[error("page {page} nests more than {MAX_NESTING} saved states and clips")]
    NestingLimit { page: usize },
    #[error(transparent)]
    Write(#[from] WriteError),
}

/// A finite coordinate, clamped to the range the writer emits.
fn coord(v: f32) -> f32 {
    v.clamp(-MAX_COORD, MAX_COORD)
}

fn point(p: [f32; 2]) -> [f32; 2] {
    [coord(p[0]), coord(p[1])]
}

/// A dash array (`[on, off, …]`, at most 8 entries), or `None` for a solid line. A pattern the PDF
/// specification forbids (a non-finite entry, or none that rounds to a positive length) is solid.
fn dash_array(dash: &[f32]) -> Option<Vec<f32>> {
    let d: Vec<f32> = dash.iter().take(8).map(|v| if v.is_finite() { v.clamp(0.0, MAX_COORD) } else { f32::NAN }).collect();
    (finite(&d) && d.iter().any(|v| (v * 1000.0).round() > 0.0)).then_some(d)
}

/// URI actions use byte strings: non-ASCII UTF-8 is percent-encoded.
fn ascii_uri(uri: &str) -> String {
    let mut ascii = String::with_capacity(uri.len());
    for byte in uri.bytes() {
        if byte.is_ascii() {
            ascii.push(byte as char);
        } else {
            ascii.push_str(&format!("%{byte:02X}"));
        }
    }
    ascii
}

fn info_string(text: &str) -> String {
    text.chars().take(MAX_INFO_CHARS).collect()
}

/// How a path is stroked.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub color: Rgb,
    /// Zero is the thinnest line the device can draw.
    pub width: f32,
    /// `None` is solid.
    pub dash: Option<Vec<f32>>,
    pub round_join: bool,
}

/// The outline of a filled or stroked path, in page coordinates (top-left origin).
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// May have a negative width or height (stroked rectangles).
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    Polyline(Vec<[f32; 2]>),
    Polygon(Vec<[f32; 2]>),
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
    },
}

/// One recorded drawing operation. Coordinates are finite and clamped.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Save,
    Restore,
    /// Intersects the clip with a rectangle until the matching [`Op::Restore`].
    ClipRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    Path {
        shape: Shape,
        fill: Option<Rgb>,
        stroke: Option<Line>,
    },
    /// `rgba` holds exactly `pw * ph * 4` bytes of straight-alpha pixels.
    Image {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        rgba: Vec<u8>,
        pw: u32,
        ph: u32,
    },
}

impl Op {
    /// Roughly the content-stream bytes this operation produces.
    fn cost(&self) -> usize {
        match self {
            Op::Save | Op::Restore => 2,
            Op::ClipRect { .. } => 48,
            Op::Image { .. } => 64,
            Op::Path { shape, .. } => {
                64 + match shape {
                    Shape::Rect { .. } => 48,
                    Shape::Circle { .. } => 200,
                    Shape::Polyline(p) | Shape::Polygon(p) => p.len().saturating_mul(24),
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LinkTarget {
    Uri(String),
    /// A location on a page of this document (zero-based index, page coordinates).
    Page {
        index: usize,
        x: f32,
        y: f32,
    },
}

/// A clickable area, in page coordinates (top-left origin), clipped to the page.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub rect: [f32; 4],
    pub target: LinkTarget,
}

/// A page under construction. Obtain one with [`Document::add_page`].
pub struct Page {
    width: f32,
    height: f32,
    ops: Vec<Op>,
    links: Vec<Link>,
    /// Estimated content-stream bytes recorded so far.
    content: usize,
    /// Clips recorded in each open save level; the first entry is the page level.
    frames: Vec<usize>,
    /// Save levels plus clips in effect.
    nesting: usize,
    overflow: Option<Overflow>,
}

#[derive(Clone, Copy, Debug)]
enum Overflow {
    Content,
    Nesting,
}

impl Page {
    fn new(width: f32, height: f32) -> Page {
        Page { width, height, ops: Vec::new(), links: Vec::new(), content: 0, frames: vec![0], nesting: 0, overflow: None }
    }

    pub fn width(&self) -> f32 {
        self.width
    }
    pub fn height(&self) -> f32 {
        self.height
    }
    /// The recorded drawing operations, in order.
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }
    pub fn links(&self) -> &[Link] {
        &self.links
    }

    /// Records `op` unless the page is over its content bound, which [`Document::finish`] reports.
    fn op(&mut self, op: Op) {
        if self.overflow.is_some() {
            return;
        }
        let content = self.content.saturating_add(op.cost());
        if content > MAX_PAGE_CONTENT {
            self.overflow = Some(Overflow::Content);
            self.ops = Vec::new();
            return;
        }
        self.content = content;
        self.ops.push(op);
    }

    fn link_rect(&self, x: f32, y: f32, w: f32, h: f32) -> Option<[f32; 4]> {
        if !finite(&[x, y, w, h]) || !extent(w) || !extent(h) || self.links.len() >= MAX_PAGE_LINKS {
            return None;
        }
        let (right, bottom) = (x + w, y + h);
        if !finite(&[right, bottom]) {
            return None;
        }
        let (left, right) = (x.max(0.0), right.min(self.width));
        let (top, bottom) = (y.max(0.0), bottom.min(self.height));
        (right > left && bottom > top).then_some([left, top, right, bottom])
    }

    /// Adds an invisible clickable area for an external URI. Invalid or excessive links are ignored.
    pub fn link_uri(&mut self, x: f32, y: f32, w: f32, h: f32, uri: &str) {
        let allowed = uri
            .split_once(':')
            .is_some_and(|(scheme, rest)| matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https" | "mailto") && !rest.is_empty());
        if !allowed || uri.len() > MAX_LINK_URI_BYTES || uri.chars().any(char::is_control) {
            return;
        }
        if let Some(rect) = self.link_rect(x, y, w, h) {
            self.links.push(Link { rect, target: LinkTarget::Uri(ascii_uri(uri)) });
        }
    }

    /// Adds a link to a location on another page of this document (zero-based page index). Links
    /// to pages that don't exist when the document is finished are dropped.
    pub fn link_page(&mut self, x: f32, y: f32, w: f32, h: f32, page: usize, target_x: f32, target_y: f32) {
        if !finite(&[target_x, target_y]) {
            return;
        }
        if let Some(rect) = self.link_rect(x, y, w, h) {
            self.links.push(Link { rect, target: LinkTarget::Page { index: page, x: coord(target_x), y: coord(target_y) } });
        }
    }

    /// Saves the graphics state (pair with [`Page::restore`]).
    pub fn save(&mut self) {
        if self.nesting >= MAX_NESTING {
            self.overflow.get_or_insert(Overflow::Nesting);
            return;
        }
        self.nesting += 1;
        self.frames.push(0);
        self.op(Op::Save);
    }

    /// Restores the graphics state saved by [`Page::save`]; extra calls are ignored.
    pub fn restore(&mut self) {
        if self.frames.len() > 1
            && let Some(clips) = self.frames.pop()
        {
            self.nesting = self.nesting.saturating_sub(clips + 1);
            self.op(Op::Restore);
        }
    }

    /// Intersects the clip with a rectangle (until the next [`Page::restore`]). Non-finite
    /// numbers count as zero and a negative extent as an empty rectangle.
    pub fn clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if self.nesting >= MAX_NESTING {
            self.overflow.get_or_insert(Overflow::Nesting);
            return;
        }
        let f = |v: f32| if v.is_finite() { coord(v) } else { 0.0 };
        self.nesting += 1;
        if let Some(clips) = self.frames.last_mut() {
            *clips += 1;
        }
        self.op(Op::ClipRect { x: f(x), y: f(y), w: f(w).max(0.0), h: f(h).max(0.0) });
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Rgb) {
        if !(extent(w) && extent(h) && finite(&[x, y])) {
            return;
        }
        self.op(Op::Path { shape: Shape::Rect { x: coord(x), y: coord(y), w: coord(w), h: coord(h) }, fill: Some(color), stroke: None });
    }

    pub fn rect_stroke(&mut self, x: f32, y: f32, w: f32, h: f32, width: f32, color: Rgb) {
        if !finite(&[x, y, w, h]) {
            return;
        }
        let stroke = Line { color, width: stroke_width(width), dash: None, round_join: false };
        self.op(Op::Path { shape: Shape::Rect { x: coord(x), y: coord(y), w: coord(w), h: coord(h) }, fill: None, stroke: Some(stroke) });
    }

    pub fn stroke_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: Rgb) {
        self.stroke_line_dashed(x0, y0, x1, y1, width, color, &[]);
    }

    /// A line with a dash pattern (`[on, off, …]` in points; empty = solid).
    pub fn stroke_line_dashed(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: Rgb, dash: &[f32]) {
        if !finite(&[x0, y0, x1, y1]) {
            return;
        }
        let stroke = Line { color, width: stroke_width(width), dash: dash_array(dash), round_join: false };
        self.op(Op::Path { shape: Shape::Polyline(vec![point([x0, y0]), point([x1, y1])]), fill: None, stroke: Some(stroke) });
    }

    /// An open polyline.
    pub fn polyline(&mut self, pts: &[[f32; 2]], width: f32, color: Rgb, dash: &[f32]) {
        if pts.len() < 2 || !pts.iter().all(|p| finite(p)) {
            return;
        }
        let stroke = Line { color, width: stroke_width(width), dash: dash_array(dash), round_join: true };
        self.op(Op::Path { shape: Shape::Polyline(pts.iter().copied().map(point).collect()), fill: None, stroke: Some(stroke) });
    }

    /// A closed polygon, filled and/or stroked.
    pub fn polygon(&mut self, pts: &[[f32; 2]], fill: Option<Rgb>, stroke: Option<(Rgb, f32)>) {
        if pts.len() < 2 || (fill.is_none() && stroke.is_none()) || !pts.iter().all(|p| finite(p)) {
            return;
        }
        let stroke = stroke.map(|(color, w)| Line { color, width: stroke_width(w), dash: None, round_join: true });
        self.op(Op::Path { shape: Shape::Polygon(pts.iter().copied().map(point).collect()), fill, stroke });
    }

    /// A circle (four cubic Béziers).
    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, fill: Option<Rgb>, stroke: Option<(Rgb, f32)>) {
        if !extent(r) || !finite(&[cx, cy]) || (fill.is_none() && stroke.is_none()) {
            return;
        }
        let stroke = stroke.map(|(color, w)| Line { color, width: stroke_width(w), dash: None, round_join: false });
        self.op(Op::Path { shape: Shape::Circle { cx: coord(cx), cy: coord(cy), r: coord(r) }, fill, stroke });
    }

    /// Draws an RGBA image (`pw`×`ph` pixels, row-major, straight alpha) into the box
    /// `(x, y, w, h)`. Malformed or oversized buffers are ignored.
    pub fn image_rgba(&mut self, x: f32, y: f32, w: f32, h: f32, rgba: &[u8], pw: u32, ph: u32) {
        let px = pw as u64 * ph as u64;
        if pw == 0 || ph == 0 || px > MAX_IMAGE_PIXELS || (rgba.len() as u64) < px * 4 || !(extent(w) && extent(h) && finite(&[x, y])) {
            return;
        }
        let Some(rgba) = rgba.get(..px as usize * 4) else { return };
        if self.overflow.is_some() {
            return;
        }
        self.op(Op::Image { x: coord(x), y: coord(y), w: coord(w), h: coord(h), rgba: rgba.to_vec(), pw, ph });
    }
}

/// A PDF document under construction.
#[derive(Default)]
pub struct Document {
    pages: Vec<Page>,
    pub info: Info,
}

impl Document {
    pub fn new() -> Document {
        Document::default()
    }

    /// Adds a page of `width_pt` × `height_pt` points (clamped to 3–14,400; non-finite becomes
    /// Letter) and returns it for drawing.
    pub fn add_page(&mut self, width_pt: f32, height_pt: f32) -> Result<&mut Page, PdfError> {
        if self.pages.len() >= MAX_PAGES {
            return Err(PdfError::PageLimit);
        }
        let width = if width_pt.is_finite() { width_pt.clamp(3.0, 14_400.0) } else { 612.0 };
        let height = if height_pt.is_finite() { height_pt.clamp(3.0, 14_400.0) } else { 792.0 };
        self.pages.push(Page::new(width, height));
        self.pages.last_mut().ok_or(PdfError::PageLimit)
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// The page at `i`, to keep drawing on it.
    pub fn page_mut(&mut self, i: usize) -> Option<&mut Page> {
        self.pages.get_mut(i)
    }

    /// Serializes the document. A document without pages gets one blank Letter page.
    pub fn finish(mut self) -> Result<Vec<u8>, PdfError> {
        for (page, p) in self.pages.iter().enumerate() {
            match p.overflow {
                Some(Overflow::Content) => return Err(PdfError::ContentLimit { page }),
                Some(Overflow::Nesting) => return Err(PdfError::NestingLimit { page }),
                None => {}
            }
        }
        if self.pages.is_empty() {
            self.pages.push(Page::new(612.0, 792.0));
        }
        let metadata = self.metadata();
        let pages = self.pages;
        write::guard(|| {
            let mut writer = Writer::new(backend::SerializeSettings::default(), metadata);
            let count = pages.len();
            for page in &pages {
                render_page(writer.document_mut(), page, count);
            }
            Ok(writer.finish()?)
        })
    }

    fn metadata(&self) -> Metadata {
        let creator = if self.info.creator.is_empty() { "GridCraft" } else { &self.info.creator };
        Metadata {
            creator: info_string(creator),
            producer: PRODUCER.into(),
            title: (!self.info.title.is_empty()).then(|| info_string(&self.info.title)),
            authors: if self.info.author.is_empty() { Vec::new() } else { vec![info_string(&self.info.author)] },
            ..Metadata::default()
        }
    }
}

fn render_page(doc: &mut backend::Document, page: &Page, page_count: usize) {
    // Clamped page sizes are always valid; Letter covers the impossible case.
    let size = Size::from_wh(page.width, page.height).or_else(|| Size::from_wh(612.0, 792.0));
    let Some(size) = size else { return };
    let mut out = doc.start_page_with(PageSettings::new(size));
    for link in &page.links {
        let [l, t, r, b] = link.rect;
        let Some(rect) = Rect::from_ltrb(l, t, r, b) else { continue };
        let target = match &link.target {
            LinkTarget::Uri(uri) => Target::Action(LinkAction::new(uri.clone()).into()),
            LinkTarget::Page { index, .. } if *index >= page_count => continue,
            LinkTarget::Page { index, x, y } => Target::Destination(XyzDestination::new(*index, Point::from_xy(*x, *y)).into()),
        };
        out.add_annotation(Annotation::new_link(LinkAnnotation::new(rect, target), None));
    }
    let mut surface = out.surface();
    draw_ops(&mut surface, &page.ops);
    surface.finish();
    out.finish();
}

/// Plays `ops` onto `surface`, leaving its push/pop stack balanced.
fn draw_ops(surface: &mut Surface<'_>, ops: &[Op]) {
    // krilla pushes for the clips of each open save level; the first entry is the page level.
    let mut frames: Vec<usize> = vec![0];
    for op in ops {
        match op {
            Op::Save => frames.push(0),
            Op::Restore => {
                if frames.len() > 1 {
                    for _ in 0..frames.pop().unwrap_or(0) {
                        surface.pop();
                    }
                }
            }
            Op::ClipRect { x, y, w, h } => {
                if let Some(path) = rect_path(*x, *y, *w, *h)
                    && let Some(clips) = frames.last_mut()
                {
                    surface.push_clip_path(&path, &FillRule::NonZero);
                    *clips += 1;
                }
            }
            Op::Path { shape, fill, stroke } => {
                let Some(path) = shape_path(shape) else { continue };
                surface.set_fill(fill.map(|c| Fill { paint: rgb::Color::new(c[0], c[1], c[2]).into(), ..Fill::default() }));
                surface.set_stroke(stroke.as_ref().map(|s| Stroke {
                    paint: rgb::Color::new(s.color[0], s.color[1], s.color[2]).into(),
                    width: s.width,
                    line_join: if s.round_join { LineJoin::Round } else { LineJoin::Miter },
                    dash: s.dash.clone().map(|array| StrokeDash { array, offset: 0.0 }),
                    ..Stroke::default()
                }));
                surface.draw_path(&path);
            }
            Op::Image { x, y, w, h, rgba, pw, ph } => {
                let Some(size) = Size::from_wh(*w, *h) else { continue };
                surface.push_transform(&Transform::from_translate(*x, *y));
                surface.draw_image(Image::from_rgba8(rgba.clone(), *pw, *ph), size);
                surface.pop();
            }
        }
    }
    for clips in frames {
        for _ in 0..clips {
            surface.pop();
        }
    }
}

/// A rectangle as a closed path; negative extents are drawn from the other corner.
fn rect_path(x: f32, y: f32, w: f32, h: f32) -> Option<Path> {
    let (x1, y1) = (coord(x + w), coord(y + h));
    let mut b = PathBuilder::new();
    b.move_to(x, y);
    b.line_to(x1, y);
    b.line_to(x1, y1);
    b.line_to(x, y1);
    b.close();
    b.finish()
}

fn shape_path(shape: &Shape) -> Option<Path> {
    match shape {
        Shape::Rect { x, y, w, h } => rect_path(*x, *y, *w, *h),
        Shape::Polyline(pts) | Shape::Polygon(pts) => {
            let (first, rest) = pts.split_first()?;
            let mut b = PathBuilder::new();
            b.move_to(first[0], first[1]);
            for p in rest {
                b.line_to(p[0], p[1]);
            }
            if matches!(shape, Shape::Polygon(_)) {
                b.close();
            }
            b.finish()
        }
        Shape::Circle { cx, cy, r } => {
            const K: f32 = 0.552_284_8;
            let (cx, cy, r) = (*cx, *cy, *r);
            let k = r * K;
            let mut b = PathBuilder::new();
            b.move_to(cx + r, cy);
            b.cubic_to(cx + r, cy + k, cx + k, cy + r, cx, cy + r);
            b.cubic_to(cx - k, cy + r, cx - r, cy + k, cx - r, cy);
            b.cubic_to(cx - r, cy - k, cx - k, cy - r, cx, cy - r);
            b.cubic_to(cx + k, cy - r, cx + r, cy - k, cx + r, cy);
            b.close();
            b.finish()
        }
    }
}

#[cfg(test)]
mod tests;
