//! Painting: turns the laid-out tree into a display list for the host's canvas, and records
//! the boxes that were painted for hit testing.
//!
//! The display list is a sequence of little-endian 32-bit words: an opcode followed by its
//! operands (floats as `f32`, colors as RGBA `u32`, strings as a byte length and UTF-8 bytes
//! padded to 4). The host replays it on a 2D canvas context.

use std::collections::HashMap;

use crate::dom::{self, Id, Kind, Node, Tree};
use crate::layout::{Rect, wrap_lines};
use crate::style::{Background, BorderStyle, Decoration, FOCUS_RING, ListStyle, Style, Transform};
use crate::text::{Fonts, Measure};

pub mod op {
    pub const FILL: u32 = 1;
    pub const STROKE: u32 = 2;
    pub const TEXT: u32 = 3;
    pub const FONT: u32 = 4;
    pub const CLIP: u32 = 5;
    pub const RESTORE: u32 = 6;
    pub const ALPHA: u32 = 7;
    pub const TRANSFORM: u32 = 8;
    pub const IMAGE: u32 = 9;
    pub const SHADOW: u32 = 10;
    pub const GRADIENT: u32 = 11;
    pub const LINE: u32 = 12;
    pub const INSET_SHADOW: u32 = 13;
    /// A 3D scene: node id, content rectangle, corner radii.
    pub const SCENE: u32 = 14;
    /// Shape of the corners of the shapes that follow, until the next CORNER (0 is round).
    pub const CORNER: u32 = 15;
    /// Film grain: rectangle, corner radii, strength.
    pub const GRAIN: u32 = 16;
}

/// A painted box, in viewport coordinates, clipped by its scroll ancestors.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub id: Id,
    pub rect: Rect,
    pub clip: Rect,
    /// For a run of text: the element whose `frags` hold it, and its index there.
    pub frag: Option<(Id, usize)>,
}

/// Text editing state of the focused field.
#[derive(Clone, Debug, Default)]
pub struct Edit {
    pub node: Option<Id>,
    /// Byte offsets of the caret and of the other end of the selection.
    pub caret: usize,
    pub anchor: usize,
    pub scroll: (f32, f32),
    pub caret_on: bool,
}

impl Edit {
    pub fn range(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }
}

#[derive(Clone, Debug)]
pub struct Popup {
    pub select: Id,
    pub hover: usize,
    pub rect: Rect,
    pub item_h: f32,
}

/// Highlight behind selected page text (translucent, so it works on light and dark pages).
const SELECTION: u32 = 0x3d8bfd66;

pub struct Painter<'a> {
    pub t: &'a Tree,
    pub fonts: &'a mut Fonts,
    pub m: &'a mut dyn Measure,
    pub images: &'a HashMap<String, (u32, f32, f32)>,
    pub edit: &'a Edit,
    pub popup: Option<&'a Popup>,
    /// Selected byte range of each text run with selected text, by (element, run index).
    pub sel: HashMap<(Id, usize), (usize, usize)>,
    pub out: Vec<u32>,
    pub hits: Vec<Hit>,
    viewport: Rect,
    font_ids: HashMap<(String, u32), u32>,
}

#[cfg(test)]
const SCREEN: Rect = Rect {
    x: -1e9,
    y: -1e9,
    w: 2e9,
    h: 2e9,
};

fn intersect(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    Rect {
        x,
        y,
        w: ((a.x + a.w).min(b.x + b.w) - x).max(0.0),
        h: ((a.y + a.h).min(b.y + b.h) - y).max(0.0),
    }
}

impl<'a> Painter<'a> {
    pub fn new(
        t: &'a Tree,
        fonts: &'a mut Fonts,
        m: &'a mut dyn Measure,
        images: &'a HashMap<String, (u32, f32, f32)>,
        edit: &'a Edit,
        popup: Option<&'a Popup>,
        viewport: Rect,
    ) -> Self {
        Painter {
            t,
            fonts,
            m,
            images,
            edit,
            popup,
            sel: HashMap::new(),
            out: Vec::new(),
            hits: Vec::new(),
            viewport,
            font_ids: HashMap::new(),
        }
    }

    fn f(&mut self, v: f32) {
        self.out.push(v.to_bits());
    }

    fn rect(&mut self, r: Rect) {
        for v in [r.x, r.y, r.w, r.h] {
            self.f(v);
        }
    }

    fn radii(&mut self, r: [f32; 4]) {
        for v in r {
            self.f(v);
        }
    }

    fn str(&mut self, s: &str) {
        self.out.push(s.len() as u32);
        for chunk in s.as_bytes().chunks(4) {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            self.out.push(u32::from_le_bytes(word));
        }
    }

    fn font(&mut self, s: &Style) -> u32 {
        let key = (s.font.to_string(), s.letter_spacing.to_bits());
        if let Some(&id) = self.font_ids.get(&key) {
            return id;
        }
        let id = self.font_ids.len() as u32;
        self.out.push(op::FONT);
        self.out.push(id);
        self.f(s.letter_spacing);
        self.str(&key.0);
        self.font_ids.insert(key, id);
        id
    }

    pub fn fill(&mut self, r: Rect, color: u32, radii: [f32; 4]) {
        if color & 0xff == 0 || r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        self.out.push(op::FILL);
        self.rect(r);
        self.out.push(color);
        self.radii(radii);
    }

    fn stroke(&mut self, r: Rect, color: u32, width: f32, radii: [f32; 4], dash: u32) {
        self.out.push(op::STROKE);
        self.rect(r);
        self.out.push(color);
        self.f(width);
        self.radii(radii);
        self.out.push(dash);
    }

    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: u32) {
        self.out.push(op::LINE);
        for v in [x1, y1, x2, y2, width] {
            self.f(v);
        }
        self.out.push(color);
    }

    /// A hand-drawn rectangle: each side twice, with wobbly, overshooting strokes that stay
    /// the same from frame to frame (seeded by the node).
    fn sketch(&mut self, id: Id, b: Rect, w: f32, color: u32) {
        let mut seed = id.wrapping_mul(2654435761).wrapping_add(1);
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed % 1000) as f32 / 500.0 - 1.0
        };
        let j = (w * 1.5).max(1.5);
        let corners = [
            (b.x, b.y),
            (b.x + b.w, b.y),
            (b.x + b.w, b.y + b.h),
            (b.x, b.y + b.h),
        ];
        for pass in 0..2 {
            for side in 0..4 {
                let (a, c) = (corners[side], corners[(side + 1) % 4]);
                let (dx, dy) = (c.0 - a.0, c.1 - a.1);
                let len = (dx * dx + dy * dy).sqrt().max(1.0);
                let (nx, ny) = (-dy / len, dx / len);
                let over = j * (1.0 + pass as f32);
                let p0 = (
                    a.0 - dx / len * over * rnd().abs() + nx * j * rnd(),
                    a.1 - dy / len * over * rnd().abs() + ny * j * rnd(),
                );
                let p2 = (
                    c.0 + dx / len * over * rnd().abs() + nx * j * rnd(),
                    c.1 + dy / len * over * rnd().abs() + ny * j * rnd(),
                );
                let bow = j * 1.5 * rnd();
                let p1 = (
                    (p0.0 + p2.0) / 2.0 + nx * bow,
                    (p0.1 + p2.1) / 2.0 + ny * bow,
                );
                // A quadratic curve through the bowed middle, in six segments.
                let at = |t: f32| {
                    let u = 1.0 - t;
                    (
                        u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0,
                        u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1,
                    )
                };
                for k in 0..6 {
                    let (s, e) = (at(k as f32 / 6.0), at((k + 1) as f32 / 6.0));
                    self.line(
                        s.0,
                        s.1,
                        e.0,
                        e.1,
                        w * if pass == 0 { 1.0 } else { 0.7 },
                        color,
                    );
                }
            }
        }
    }

    /// Draws `text` with its vertical middle at `mid`.
    pub fn text(&mut self, s: &Style, x: f32, mid: f32, text: &str, color: u32) {
        if text.is_empty() || color & 0xff == 0 {
            return;
        }
        let font = self.font(s);
        self.out.push(op::TEXT);
        self.f(x);
        self.f(mid);
        self.out.push(color);
        self.out.push(font);
        self.str(text);
    }

    fn clip(&mut self, r: Rect, radii: [f32; 4]) {
        self.out.push(op::CLIP);
        self.rect(r);
        self.radii(radii);
    }

    fn restore(&mut self) {
        self.out.push(op::RESTORE);
    }

    /// Paints the tree; the root is scrolled by its own scroll offset.
    pub fn paint(&mut self) {
        let root = self.t.n(0);
        self.node(0, 0.0, -root.lay.scroll.1, self.viewport);
        if let Some(p) = self.popup.cloned() {
            self.popup(&p);
        }
    }

    fn node(&mut self, id: Id, ox: f32, oy: f32, clip: Rect) {
        let n = self.t.n(id);
        let s = &n.style;
        if n.kind != Kind::Element || s.display == crate::style::Display::None {
            return;
        }
        if s.display == crate::style::Display::Contents {
            for &c in &n.children.clone() {
                self.node(c, ox, oy, clip);
            }
            return;
        }
        let fixed = s.position == crate::style::Position::Fixed;
        let (ox, oy, clip) = if fixed {
            (0.0, 0.0, self.viewport)
        } else {
            (ox, oy, clip)
        };
        let r = n.lay.rect;
        let x = if id == 0 {
            0.0
        } else {
            ox + r.x + n.lay.offset.0
        };
        let y = if id == 0 {
            oy
        } else {
            oy + r.y + n.lay.offset.1
        };
        let b = Rect {
            x,
            y,
            w: r.w,
            h: r.h,
        };
        // Skip subtrees that paint nothing inside the visible area (4px for focus rings).
        let e = n.lay.extent;
        let ink = Rect {
            x: x + e.x - 4.0,
            y: y + e.y - 4.0,
            w: e.w + 8.0,
            h: e.h + 8.0,
        };
        if id != 0 && intersect(ink, clip).w * intersect(ink, clip).h <= 0.0 {
            return;
        }
        let mut saves = 0;
        if s.opacity < 1.0 {
            self.out.push(op::ALPHA);
            self.f(s.opacity);
            saves += 1;
        }
        let mut hit_rect = b;
        if !s.transform.is_empty() {
            let m = matrix(&s.transform, b);
            self.out.push(op::TRANSFORM);
            for v in m {
                self.f(v);
            }
            saves += 1;
            hit_rect = bounds(m, b);
        }
        self.hits.push(Hit {
            id,
            rect: hit_rect,
            clip,
            frag: None,
        });
        if s.corner != 0 {
            self.out.extend([op::CORNER, s.corner as u32]);
        }
        self.decorate(id, n, b);
        let lay = &n.lay;
        let clips = s.clips() || n.is("input") || n.is("select");
        let inner_clip = if clips {
            let [bt, br, bb, bl] = lay.border;
            let pad = Rect {
                x: b.x + bl,
                y: b.y + bt,
                w: b.w - bl - br,
                h: b.h - bt - bb,
            };
            let radii = s.radius.map(|v| (v - bt.max(bl)).max(0.0));
            self.clip(pad, radii);
            intersect(clip, pad)
        } else {
            clip
        };
        if s.corner != 0 {
            self.out.extend([op::CORNER, 0]);
        }
        let (cx, cy) = (
            x - lay.scroll.0,
            y - if id == 0 { 0.0 } else { lay.scroll.1 },
        );
        match n.tag.as_str() {
            "input" if n.attr("type") == Some("checkbox") => self.checkbox(n, b),
            "input" | "textarea" => self.field(id, n, b),
            "select" => self.select(n, b),
            "img" => self.image(n, b),
            "scene" => {
                let lay = &n.lay;
                let c = Rect {
                    x: b.x + lay.border[3] + lay.padding[3],
                    y: b.y + lay.border[0] + lay.padding[0],
                    ..lay.content_box()
                };
                let radii = n.style.radius.map(|v| (v - lay.border[0]).max(0.0));
                self.out.push(op::SCENE);
                self.out.push(id);
                self.rect(c);
                self.out.extend(radii.map(f32::to_bits));
            }
            _ => {}
        }
        for (fi, f) in lay.frags.iter().enumerate() {
            let ts = &self.t.n(f.node).style;
            let fx = cx + f.rect.x;
            let fy = cy + f.rect.y;
            let style = ts.clone();
            if let Some(&(a, z)) = self.sel.get(&(id, fi)) {
                let x1 = self.fonts.width(self.m, &style, &f.text[..a]);
                let x2 = self.fonts.width(self.m, &style, &f.text[..z]);
                let r = Rect {
                    x: fx + x1,
                    y: fy,
                    w: (x2 - x1).max(4.0),
                    h: f.rect.h,
                };
                self.fill(r, SELECTION, [0.0; 4]);
            }
            self.text(&style, fx, fy + f.rect.h / 2.0, &f.text, style.color);
            match style.decoration {
                Decoration::Underline => {
                    let uy = fy + f.rect.h / 2.0 + style.font_size * 0.42;
                    self.line(
                        fx,
                        uy,
                        fx + f.rect.w,
                        uy,
                        (style.font_size / 16.0).max(1.0),
                        style.color,
                    );
                }
                Decoration::LineThrough => {
                    let ly = fy + f.rect.h / 2.0 + style.font_size * 0.05;
                    self.line(
                        fx,
                        ly,
                        fx + f.rect.w,
                        ly,
                        (style.font_size / 16.0).max(1.0),
                        style.color,
                    );
                }
                Decoration::None => {}
            }
            self.hits.push(Hit {
                frag: Some((id, fi)),
                id: f.node,
                rect: Rect {
                    x: fx,
                    y: fy,
                    w: f.rect.w,
                    h: f.rect.h,
                },
                clip: inner_clip,
            });
        }
        let mut kids: Vec<Id> = n
            .children
            .iter()
            .copied()
            .filter(|&c| self.t.n(c).kind == Kind::Element)
            .collect();
        kids.sort_by_key(|&c| self.t.n(c).style.z);
        let mut marker = 0;
        for c in kids {
            if self.t.n(c).is("li") {
                marker += 1;
                self.marker(c, cx, cy, marker, inner_clip);
            }
            self.node(c, cx, cy, inner_clip);
        }
        if clips {
            self.restore();
            if s.clips() && id != 0 {
                self.scrollbar(n, b);
            }
        }
        if n.flags & dom::FOCUS_VISIBLE != 0 {
            let o = Rect {
                x: b.x - 3.0,
                y: b.y - 3.0,
                w: b.w + 6.0,
                h: b.h + 6.0,
            };
            let radii = s.radius.map(|v| if v > 0.0 { v + 3.0 } else { 0.0 });
            self.stroke(o, FOCUS_RING, 2.0, radii, 0);
        }
        for _ in 0..saves {
            self.restore();
        }
    }

    /// Shadows, background and borders.
    fn decorate(&mut self, id: Id, n: &Node, b: Rect) {
        let s = &n.style;
        for sh in s.shadow.iter().filter(|sh| !sh.inset) {
            self.out.push(op::SHADOW);
            let r = Rect {
                x: b.x - sh.spread,
                y: b.y - sh.spread,
                w: b.w + 2.0 * sh.spread,
                h: b.h + 2.0 * sh.spread,
            };
            self.rect(r);
            self.radii(s.radius);
            for v in [sh.x, sh.y, sh.blur] {
                self.f(v);
            }
            self.out.push(sh.color);
        }
        match &s.background {
            Background::None => {}
            Background::Color(c) => self.fill(b, *c, s.radius),
            Background::Linear(angle, stops) => {
                let a = angle.to_radians();
                let (dx, dy) = (a.sin(), -a.cos());
                let half = (b.w * dx.abs() + b.h * dy.abs()) / 2.0;
                let (mx, my) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
                self.out.push(op::GRADIENT);
                self.rect(b);
                self.radii(s.radius);
                for v in [
                    mx - dx * half,
                    my - dy * half,
                    mx + dx * half,
                    my + dy * half,
                ] {
                    self.f(v);
                }
                self.out.push(stops.len() as u32);
                for &(at, c) in stops {
                    self.f(at);
                    self.out.push(c);
                }
            }
        }
        // Inset shadows go above the background, below the border.
        for sh in s.shadow.iter().filter(|sh| sh.inset) {
            self.out.push(op::INSET_SHADOW);
            self.rect(b);
            self.radii(s.radius);
            for v in [sh.x, sh.y, sh.blur, sh.spread] {
                self.f(v);
            }
            self.out.push(sh.color);
        }
        if s.grain > 0.0 {
            self.out.push(op::GRAIN);
            self.rect(b);
            self.radii(s.radius);
            self.f(s.grain);
        }
        if let Some(sk) = s.sketch {
            self.sketch(id, b, sk.width, sk.color.unwrap_or(s.color));
            return;
        }
        let bs = s.border;
        let uniform = bs
            .iter()
            .all(|x| x.width == bs[0].width && x.color == bs[0].color && x.style == bs[0].style);
        if uniform && bs[0].width > 0.0 {
            let w = bs[0].width;
            let r = Rect {
                x: b.x + w / 2.0,
                y: b.y + w / 2.0,
                w: b.w - w,
                h: b.h - w,
            };
            let dash = match bs[0].style {
                BorderStyle::Dashed => 1,
                BorderStyle::Dotted => 2,
                _ => 0,
            };
            self.stroke(
                r,
                bs[0].color.unwrap_or(s.color),
                w,
                s.radius.map(|v| (v - w / 2.0).max(0.0)),
                dash,
            );
        } else if !uniform {
            let [t, rt, bt, l] = bs;
            let c = |x: crate::style::Border| x.color.unwrap_or(s.color);
            self.fill(
                Rect {
                    x: b.x,
                    y: b.y,
                    w: b.w,
                    h: t.width,
                },
                c(t),
                [0.0; 4],
            );
            self.fill(
                Rect {
                    x: b.x,
                    y: b.y + b.h - bt.width,
                    w: b.w,
                    h: bt.width,
                },
                c(bt),
                [0.0; 4],
            );
            self.fill(
                Rect {
                    x: b.x,
                    y: b.y,
                    w: l.width,
                    h: b.h,
                },
                c(l),
                [0.0; 4],
            );
            self.fill(
                Rect {
                    x: b.x + b.w - rt.width,
                    y: b.y,
                    w: rt.width,
                    h: b.h,
                },
                c(rt),
                [0.0; 4],
            );
        }
    }

    fn marker(&mut self, li: Id, cx: f32, cy: f32, index: usize, clip: Rect) {
        let n = self.t.n(li);
        let r = n.lay.rect;
        // Like their items, markers outside the visible area are not painted.
        if cy + r.y > clip.y + clip.h || cy + r.y + r.h < clip.y {
            return;
        }
        let s = n.style.clone();
        let mid = cy + r.y + n.lay.border[0] + n.lay.padding[0] + s.line_height / 2.0;
        match s.list_style {
            ListStyle::None => {}
            ListStyle::Decimal => {
                let label = format!("{index}.");
                let w = self.fonts.width(self.m, &s, &label);
                self.text(&s, cx + r.x - w - 6.0, mid, &label, s.color);
            }
            style => {
                let d = (s.font_size * 0.35).round();
                let m = Rect {
                    x: cx + r.x - d - 8.0,
                    y: mid - d / 2.0,
                    w: d,
                    h: d,
                };
                match style {
                    ListStyle::Square => self.fill(m, s.color, [0.0; 4]),
                    ListStyle::Circle => self.stroke(m, s.color, 1.0, [d / 2.0; 4], 0),
                    _ => self.fill(m, s.color, [d / 2.0; 4]),
                }
            }
        }
    }

    fn scrollbar(&mut self, n: &Node, b: Rect) {
        let lay = &n.lay;
        let (_, my) = lay.max_scroll();
        if my <= 0.0 {
            return;
        }
        let track = b.h - lay.border[0] - lay.border[2];
        let thumb = (track * track / lay.content.1).max(20.0);
        let y = b.y + lay.border[0] + (track - thumb) * lay.scroll.1 / my;
        self.fill(
            Rect {
                x: b.x + b.w - lay.border[1] - 7.0,
                y: y + 2.0,
                w: 5.0,
                h: thumb - 4.0,
            },
            0x00000066,
            [2.5; 4],
        );
    }

    fn checkbox(&mut self, n: &Node, b: Rect) {
        let checked = n.checked;
        let disabled = n.attr("disabled").is_some();
        let accent = if disabled { 0x9e9e9eff } else { 0x0b57d0ff };
        if checked {
            self.fill(b, accent, [2.0; 4]);
            let (x, y, w, h) = (b.x, b.y, b.w, b.h);
            self.line(
                x + w * 0.22,
                y + h * 0.52,
                x + w * 0.42,
                y + h * 0.72,
                2.0,
                0xffffffff,
            );
            self.line(
                x + w * 0.42,
                y + h * 0.72,
                x + w * 0.78,
                y + h * 0.3,
                2.0,
                0xffffffff,
            );
        } else {
            self.fill(b, 0xffffffff, [2.0; 4]);
            let r = Rect {
                x: b.x + 0.5,
                y: b.y + 0.5,
                w: b.w - 1.0,
                h: b.h - 1.0,
            };
            self.stroke(
                r,
                if n.flags & dom::HOVER != 0 {
                    0x4f4f4fff
                } else {
                    0x767676ff
                },
                1.0,
                [2.0; 4],
                0,
            );
        }
    }

    fn field(&mut self, id: Id, n: &Node, b: Rect) {
        let s = n.style.clone();
        let lay = &n.lay;
        let content = Rect {
            x: b.x + lay.border[3] + lay.padding[3],
            y: b.y + lay.border[0] + lay.padding[0],
            ..lay.content_box()
        };
        let editing = self.edit.node == Some(id);
        let scroll = if editing {
            self.edit.scroll
        } else {
            (0.0, 0.0)
        };
        let value = n.value.clone();
        if value.is_empty()
            && let Some(ph) = n.attr("placeholder")
        {
            let ph = ph.to_string();
            self.text(
                &s,
                content.x,
                content.y + s.line_height / 2.0,
                &ph,
                0x757575ff,
            );
        }
        let lines: Vec<(usize, usize)> = if n.is("textarea") {
            wrap_lines(self.fonts, self.m, &s, &value, content.w)
        } else {
            vec![(0, value.len())]
        };
        let (sel_a, sel_b) = if editing { self.edit.range() } else { (0, 0) };
        for (i, &(a, z)) in lines.iter().enumerate() {
            let top = content.y + i as f32 * s.line_height - scroll.1;
            if top + s.line_height < b.y || top > b.y + b.h {
                continue;
            }
            let x0 = content.x - scroll.0;
            if editing && sel_a != sel_b && sel_a <= z && sel_b >= a {
                let from = sel_a.max(a);
                let to = sel_b.min(z);
                let x1 = x0 + self.fonts.width(self.m, &s, &value[a..from]);
                let x2 = x0 + self.fonts.width(self.m, &s, &value[a..to]);
                let extra = if sel_b > z && i + 1 < lines.len() {
                    4.0
                } else {
                    0.0
                };
                self.fill(
                    Rect {
                        x: x1,
                        y: top,
                        w: x2 - x1 + extra,
                        h: s.line_height,
                    },
                    0xb4d5feff,
                    [0.0; 4],
                );
            }
            let color = if n.attr("disabled").is_some() {
                0x6d6d6dff
            } else {
                s.color
            };
            self.text(&s, x0, top + s.line_height / 2.0, &value[a..z], color);
        }
        if editing && self.edit.caret_on && sel_a == sel_b {
            let (line, x) = caret_xy(self.fonts, self.m, &s, &value, &lines, self.edit.caret);
            let cx = content.x - scroll.0 + x;
            let top = content.y + line as f32 * s.line_height - scroll.1;
            self.fill(
                Rect {
                    x: cx.round(),
                    y: top + 2.0,
                    w: 1.0,
                    h: s.line_height - 4.0,
                },
                s.color,
                [0.0; 4],
            );
        }
    }

    fn select(&mut self, n: &Node, b: Rect) {
        let s = n.style.clone();
        let lay = &n.lay;
        let x = b.x + lay.border[3] + lay.padding[3];
        let text = n
            .children
            .iter()
            .copied()
            .find(|&o| self.t.n(o).attr("value") == Some(n.value.as_str()))
            .or(n.children.first().copied())
            .map(|o| self.t.text_of(o))
            .unwrap_or_default();
        let mid = b.y + b.h / 2.0;
        self.text(&s, x, mid, &text, s.color);
        let ax = b.x + b.w - lay.border[1] - 12.0;
        self.line(ax - 4.0, mid - 2.0, ax, mid + 2.0, 1.5, s.color);
        self.line(ax, mid + 2.0, ax + 4.0, mid - 2.0, 1.5, s.color);
    }

    fn popup(&mut self, p: &Popup) {
        let sel = self.t.n(p.select);
        let s = sel.style.clone();
        let r = p.rect;
        self.out.push(op::SHADOW);
        self.rect(r);
        self.radii([4.0; 4]);
        for v in [0.0, 2.0, 8.0] {
            self.f(v);
        }
        self.out.push(0x00000040);
        self.fill(r, 0xffffffff, [4.0; 4]);
        self.stroke(
            Rect {
                x: r.x + 0.5,
                y: r.y + 0.5,
                w: r.w - 1.0,
                h: r.h - 1.0,
            },
            0x767676ff,
            1.0,
            [4.0; 4],
            0,
        );
        let opts: Vec<(String, bool)> = sel
            .children
            .iter()
            .map(|&o| {
                (
                    self.t.text_of(o),
                    self.t.n(o).attr("value") == Some(sel.value.as_str()),
                )
            })
            .collect();
        for (i, (label, selected)) in opts.iter().enumerate() {
            let row = Rect {
                x: r.x + 1.0,
                y: r.y + 1.0 + i as f32 * p.item_h,
                w: r.w - 2.0,
                h: p.item_h,
            };
            let fg = if i == p.hover {
                self.fill(row, 0x0b57d0ff, [0.0; 4]);
                0xffffffff
            } else {
                if *selected {
                    self.fill(row, 0xe8f0feff, [0.0; 4]);
                }
                0x000000ff
            };
            self.text(&s, row.x + 8.0, row.y + row.h / 2.0, label, fg);
        }
    }

    fn image(&mut self, n: &Node, b: Rect) {
        let Some(&(img, iw, ih)) = n.attr("src").and_then(|s| self.images.get(s)) else {
            return;
        };
        if iw <= 0.0 || ih <= 0.0 {
            return;
        }
        let lay = &n.lay;
        let c = Rect {
            x: b.x + lay.border[3] + lay.padding[3],
            y: b.y + lay.border[0] + lay.padding[0],
            ..lay.content_box()
        };
        let (mut dst, mut src) = (
            c,
            Rect {
                x: 0.0,
                y: 0.0,
                w: iw,
                h: ih,
            },
        );
        match n.style.fit {
            crate::style::Fit::Fill => {}
            crate::style::Fit::Contain => {
                let k = (c.w / iw).min(c.h / ih);
                dst = Rect {
                    x: c.x + (c.w - iw * k) / 2.0,
                    y: c.y + (c.h - ih * k) / 2.0,
                    w: iw * k,
                    h: ih * k,
                };
            }
            crate::style::Fit::Cover => {
                let k = (c.w / iw).max(c.h / ih);
                src = Rect {
                    x: (iw - c.w / k) / 2.0,
                    y: (ih - c.h / k) / 2.0,
                    w: c.w / k,
                    h: c.h / k,
                };
            }
            crate::style::Fit::None => {
                src = Rect {
                    x: ((iw - c.w) / 2.0).max(0.0),
                    y: ((ih - c.h) / 2.0).max(0.0),
                    w: iw.min(c.w),
                    h: ih.min(c.h),
                };
                dst = Rect {
                    x: c.x + ((c.w - iw) / 2.0).max(0.0),
                    y: c.y + ((c.h - ih) / 2.0).max(0.0),
                    w: src.w,
                    h: src.h,
                };
            }
        }
        self.out.push(op::IMAGE);
        self.out.push(img);
        self.rect(dst);
        self.rect(src);
    }
}

/// Line index and x offset of the caret at byte `caret`.
pub fn caret_xy(
    fonts: &mut Fonts,
    m: &mut dyn Measure,
    s: &Style,
    value: &str,
    lines: &[(usize, usize)],
    caret: usize,
) -> (usize, f32) {
    let line = lines.iter().rposition(|&(a, _)| a <= caret).unwrap_or(0);
    let (a, z) = lines[line];
    (line, fonts.width(m, s, &value[a..caret.min(z).max(a)]))
}

/// The 2D affine matrix (a, b, c, d, e, f) for `transform` around the box center.
fn matrix(ts: &[Transform], b: Rect) -> [f32; 6] {
    let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
    let mut m = [1.0, 0.0, 0.0, 1.0, cx, cy];
    let mul = |m: [f32; 6], n: [f32; 6]| {
        [
            m[0] * n[0] + m[2] * n[1],
            m[1] * n[0] + m[3] * n[1],
            m[0] * n[2] + m[2] * n[3],
            m[1] * n[2] + m[3] * n[3],
            m[0] * n[4] + m[2] * n[5] + m[4],
            m[1] * n[4] + m[3] * n[5] + m[5],
        ]
    };
    for t in ts {
        let n = match *t {
            Transform::Translate(x, y) => [
                1.0,
                0.0,
                0.0,
                1.0,
                x.px(Some(b.w)).unwrap_or(0.0),
                y.px(Some(b.h)).unwrap_or(0.0),
            ],
            Transform::Scale(x, y) => [x, 0.0, 0.0, y, 0.0, 0.0],
            Transform::Rotate(d) => {
                let (s, c) = d.to_radians().sin_cos();
                [c, s, -s, c, 0.0, 0.0]
            }
        };
        m = mul(m, n);
    }
    mul(m, [1.0, 0.0, 0.0, 1.0, -cx, -cy])
}

fn bounds(m: [f32; 6], b: Rect) -> Rect {
    let pts = [
        (b.x, b.y),
        (b.x + b.w, b.y),
        (b.x, b.y + b.h),
        (b.x + b.w, b.y + b.h),
    ]
    .map(|(x, y)| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]));
    let (x0, x1) = pts
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, z), p| (a.min(p.0), z.max(p.0)));
    let (y0, y1) = pts
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, z), p| (a.min(p.1), z.max(p.1)));
    Rect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    }
}

/// The topmost painted node at a viewport point.
pub fn hit_test(hits: &[Hit], x: f32, y: f32) -> Option<Id> {
    hits.iter()
        .rev()
        .find(|h| h.rect.contains(x, y) && h.clip.contains(x, y))
        .map(|h| h.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms() {
        let b = Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        };
        let m = matrix(&[Transform::Scale(2.0, 2.0)], b);
        assert_eq!(
            bounds(m, b),
            Rect {
                x: -5.0,
                y: -5.0,
                w: 20.0,
                h: 20.0
            }
        );
        let m = matrix(
            &[Transform::Translate(
                crate::style::Len::Px(3.0),
                crate::style::Len::Pct(50.0),
            )],
            b,
        );
        assert_eq!(
            bounds(m, b),
            Rect {
                x: 3.0,
                y: 5.0,
                w: 10.0,
                h: 10.0
            }
        );
    }

    #[test]
    fn hits_respect_order_and_clip() {
        let hits = [
            Hit {
                id: 1,
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 100.0,
                    h: 100.0,
                },
                clip: SCREEN,
                frag: None,
            },
            Hit {
                id: 2,
                rect: Rect {
                    x: 10.0,
                    y: 10.0,
                    w: 50.0,
                    h: 50.0,
                },
                clip: SCREEN,
                frag: None,
            },
            Hit {
                id: 3,
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 100.0,
                    h: 100.0,
                },
                clip: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 5.0,
                    h: 5.0,
                },
                frag: None,
            },
        ];
        assert_eq!(hit_test(&hits, 20.0, 20.0), Some(2));
        assert_eq!(hit_test(&hits, 2.0, 2.0), Some(3));
        assert_eq!(hit_test(&hits, 80.0, 80.0), Some(1));
        assert_eq!(hit_test(&hits, 200.0, 200.0), None);
    }
}
