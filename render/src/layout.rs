//! Layout: block flow with inline text, flexbox, grid and absolute positioning.
//!
//! Every element gets a border box relative to its parent's border box. Text is laid out in
//! fragments owned by the element that contains it. Sizes use `box-sizing: border-box`.

use std::collections::HashMap;

use crate::dom::{Id, Kind, Tree};
use crate::style::{Align, Display, Justify, Len, Position, Style, TextAlign, Tracks};
use crate::text::{self, Fonts, Measure, Token};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

/// A run of text on one line, in the owning element's coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Frag {
    /// The text node the run comes from.
    pub node: Id,
    pub rect: Rect,
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct Lay {
    pub rect: Rect,
    /// Offset from `position: relative`, applied when painting.
    pub offset: (f32, f32),
    pub frags: Vec<Frag>,
    /// Size of the content of a scroll container, including padding.
    pub content: (f32, f32),
    pub scroll: (f32, f32),
    pub padding: [f32; 4],
    pub border: [f32; 4],
    /// Bounding box of everything the subtree paints, relative to `rect`'s origin.
    pub extent: Rect,
    /// Inputs and result of the layout that produced the current child positions.
    cache: Option<(f32, Option<f32>, f32)>,
    /// Heights for other inputs, from measuring passes.
    measures: Vec<(f32, Option<f32>, f32)>,
    intrinsic: Option<(f32, f32)>,
}

impl Lay {
    /// Forgets cached results; returns false if there were none.
    pub fn invalidate(&mut self) -> bool {
        let had = self.cache.is_some() || self.intrinsic.is_some() || !self.measures.is_empty();
        self.cache = None;
        self.intrinsic = None;
        self.measures.clear();
        had
    }

    pub fn content_width(&self) -> Option<f32> {
        (self.rect.w > 0.0).then(|| {
            self.rect.w - self.padding[1] - self.padding[3] - self.border[1] - self.border[3]
        })
    }

    /// The content box in the element's own coordinates.
    pub fn content_box(&self) -> Rect {
        let [pt, pr, pb, pl] = self.padding;
        let [bt, br, bb, bl] = self.border;
        Rect {
            x: bl + pl,
            y: bt + pt,
            w: (self.rect.w - bl - pl - pr - br).max(0.0),
            h: (self.rect.h - bt - pt - pb - bb).max(0.0),
        }
    }

    pub fn max_scroll(&self) -> (f32, f32) {
        let [_, br, bb, bl] = self.border;
        let bt = self.border[0];
        (
            (self.content.0 - (self.rect.w - bl - br)).max(0.0),
            (self.content.1 - (self.rect.h - bt - bb)).max(0.0),
        )
    }
}

pub struct Cx<'a> {
    pub t: &'a mut Tree,
    pub m: &'a mut dyn Measure,
    pub fonts: &'a mut Fonts,
    pub images: &'a HashMap<String, (f32, f32)>,
    pub vw: f32,
    pub vh: f32,
}

#[derive(Clone, Copy)]
enum Item {
    Elem(Id),
    Text(Id),
}

enum Piece {
    Text {
        node: Id,
        s: String,
        w: f32,
        space: bool,
        wrap: bool,
    },
    Atom {
        id: Id,
        w: f32,
        margin: [f32; 4],
    },
    Break,
}

const INF: f32 = f32::INFINITY;

/// Like `f32::clamp`, without its panic (and the float formatting it pulls in).
pub fn clamp(v: f32, min: f32, max: f32) -> f32 {
    v.min(max).max(min)
}

/// Lays out the tree for a viewport of `vw` × `vh`. Subtrees whose layout was not
/// invalidated (see `Tree::mark_layout`) reuse their previous results.
pub fn layout_root(cx: &mut Cx) {
    let h = cx.layout(0, cx.vw, None);
    let root = &mut cx.t.m(0).lay;
    root.rect = Rect {
        x: 0.0,
        y: 0.0,
        w: cx.vw,
        h: h.max(cx.vh),
    };
    root.content.1 = root.content.1.max(h);
    let max = root.max_scroll();
    root.scroll.1 = clamp(
        root.scroll.1,
        0.0,
        (h - cx.vh).max(0.0).min(max.1.max(h - cx.vh)),
    );
}

impl Cx<'_> {
    fn s(&self, id: Id) -> &Style {
        &self.t.n(id).style
    }

    fn pb(&self, id: Id, cb_w: f32) -> ([f32; 4], [f32; 4]) {
        let s = self.s(id);
        let p = s.padding.map(|l| l.px(Some(cb_w)).unwrap_or(0.0));
        let b = s.border.map(|b| {
            if b.style == crate::style::BorderStyle::Solid || b.width > 0.0 {
                b.width
            } else {
                0.0
            }
        });
        (p, b)
    }

    fn margins(&self, id: Id, cb_w: f32) -> ([f32; 4], [bool; 4]) {
        let m = self.s(id).margin;
        (
            m.map(|l| l.px(Some(cb_w)).unwrap_or(0.0)),
            m.map(|l| l == Len::Auto),
        )
    }

    /// In-flow children with `display: contents` flattened; absolutely positioned ones excluded.
    fn items(&self, id: Id, out: &mut Vec<Item>) {
        for &c in &self.t.n(id).children {
            let n = self.t.n(c);
            match n.kind {
                Kind::Comment => {}
                Kind::Text if n.text.is_empty() => {}
                Kind::Text => out.push(Item::Text(c)),
                Kind::Element => match n.style.display {
                    Display::None => {}
                    Display::Contents => self.items(c, out),
                    _ if matches!(n.style.position, Position::Absolute | Position::Fixed) => {}
                    _ => out.push(Item::Elem(c)),
                },
            }
        }
    }

    fn out_of_flow(&self, id: Id, out: &mut Vec<Id>) {
        for &c in &self.t.n(id).children {
            let n = self.t.n(c);
            if n.kind != Kind::Element || n.style.display == Display::None {
                continue;
            }
            if n.style.display == Display::Contents {
                self.out_of_flow(c, out);
            } else if matches!(n.style.position, Position::Absolute | Position::Fixed) {
                out.push(c);
            }
        }
    }

    fn inline_level(&self, item: Item) -> bool {
        match item {
            Item::Text(_) => true,
            Item::Elem(id) => self.s(id).display.inline_level(),
        }
    }

    /// Replaced elements size their own content instead of laying out children.
    fn replaced(&self, id: Id) -> bool {
        let n = self.t.n(id);
        n.kind == Kind::Element && matches!(n.tag.as_str(), "input" | "textarea" | "select" | "img")
    }

    /// Content-box size of a replaced element for a given content width (if known).
    fn replaced_size(&mut self, id: Id, width: Option<f32>) -> (f32, f32) {
        let n = self.t.n(id);
        let lh = n.style.line_height;
        match n.tag.as_str() {
            "img" => {
                let (iw, ih) = n
                    .attr("src")
                    .and_then(|s| self.images.get(s))
                    .copied()
                    .unwrap_or((0.0, 0.0));
                match width {
                    Some(w) if iw > 0.0 => (w, w * ih / iw),
                    _ => (iw, ih),
                }
            }
            "input" if n.attr("type") == Some("checkbox") => (0.0, 0.0),
            "input" => (width.unwrap_or(147.0), lh),
            "textarea" => {
                let rows = n
                    .attr("rows")
                    .and_then(|r| r.parse::<f32>().ok())
                    .unwrap_or(2.0)
                    .max(1.0);
                (width.unwrap_or(180.0), rows * lh)
            }
            _ => {
                let s = n.style.clone();
                let labels: Vec<String> = n.children.iter().map(|&o| self.t.text_of(o)).collect();
                let w = labels
                    .iter()
                    .map(|l| self.fonts.width(self.m, &s, l))
                    .fold(0.0, f32::max);
                (width.unwrap_or(w + 20.0), lh)
            }
        }
    }

    /// Border-box width from an explicit `width`, clamped by `min-width`/`max-width`.
    fn explicit_width(&self, id: Id, cb_w: f32) -> Option<f32> {
        self.s(id)
            .width
            .px(Some(cb_w))
            .map(|w| self.clamp_w(id, w, cb_w))
    }

    fn clamp_w(&self, id: Id, w: f32, cb_w: f32) -> f32 {
        let s = self.s(id);
        let min = s.min_width.px(Some(cb_w)).unwrap_or(0.0);
        let max = s.max_width.px(Some(cb_w)).unwrap_or(INF);
        clamp(w, min, max)
    }

    fn clamp_h(&self, id: Id, h: f32, cb_h: Option<f32>) -> f32 {
        let s = self.s(id);
        let min = s.min_height.px(cb_h).unwrap_or(0.0);
        let max = s.max_height.px(cb_h).unwrap_or(INF);
        clamp(h, min, max)
    }

    /// Border-box height from an explicit `height`, if it resolves.
    fn fixed_h(&self, id: Id, cb_h: Option<f32>) -> Option<f32> {
        self.s(id)
            .height
            .px(cb_h)
            .map(|h| self.clamp_h(id, h, cb_h))
    }

    /// Shrink-to-fit width: the preferred width, but no wider than `avail`.
    fn fit_width(&mut self, id: Id, avail: f32) -> f32 {
        if let Some(w) = self.explicit_width(id, avail) {
            return w;
        }
        let (min, max) = self.intrinsic(id);
        self.clamp_w(id, max.min(avail).max(min), avail)
    }

    /// Minimum and maximum content widths of the border box.
    pub fn intrinsic(&mut self, id: Id) -> (f32, f32) {
        if let Some(v) = self.t.n(id).lay.intrinsic {
            return v;
        }
        let v = self.intrinsic_of(id, true);
        self.t.m(id).lay.intrinsic = Some(v);
        v
    }

    /// Intrinsic widths; with `own_width` false, an explicit `width` is ignored (the content
    /// size used for flex items' automatic minimum).
    fn intrinsic_of(&mut self, id: Id, own_width: bool) -> (f32, f32) {
        let (p, b) = self.pb(id, 0.0);
        let extra = p[1] + p[3] + b[1] + b[3];
        let s = self.s(id).clone();
        let (min, max) = if let (Len::Px(w), true) = (s.width, own_width) {
            (w - extra, w - extra)
        } else if self.replaced(id) {
            let (w, _) = self.replaced_size(id, None);
            (w, w)
        } else {
            let mut items = Vec::new();
            self.items(id, &mut items);
            let mut kids: Vec<(f32, f32, f32)> = Vec::new();
            let mut run: Vec<Item> = Vec::new();
            let flex_or_grid = matches!(
                s.display,
                Display::Flex | Display::InlineFlex | Display::Grid
            );
            for it in items {
                if !flex_or_grid && self.inline_level(it) {
                    run.push(it);
                    continue;
                }
                if !run.is_empty() {
                    let (a, b) = self.run_intrinsic(&std::mem::take(&mut run));
                    kids.push((a, b, 0.0));
                }
                let (a, b, m) = match it {
                    Item::Text(_) => {
                        let (a, b) = self.run_intrinsic(&[it]);
                        (a, b, 0.0)
                    }
                    Item::Elem(c) => {
                        let (a, b) = self.intrinsic(c);
                        let (m, _) = self.margins(c, 0.0);
                        (a, b, m[1] + m[3])
                    }
                };
                kids.push((a, b, m));
            }
            if !run.is_empty() {
                let (a, b) = self.run_intrinsic(&run);
                kids.push((a, b, 0.0));
            }
            let gaps = s.col_gap * kids.len().saturating_sub(1) as f32;
            match s.display {
                Display::Flex | Display::InlineFlex if s.row => {
                    let max = kids.iter().map(|k| k.1 + k.2).sum::<f32>() + gaps;
                    let min = if s.wrap {
                        kids.iter().map(|k| k.0 + k.2).fold(0.0, f32::max)
                    } else {
                        kids.iter().map(|k| k.0 + k.2).sum::<f32>() + gaps
                    };
                    (min, max)
                }
                Display::Grid if !s.stack => {
                    let n = match &s.columns {
                        Tracks::Count(n) => *n as f32,
                        Tracks::List(l) => l.len().max(1) as f32,
                        _ => 1.0,
                    };
                    let g = s.col_gap * (n - 1.0);
                    let widest = kids.iter().map(|k| k.1 + k.2).fold(0.0, f32::max);
                    let min = kids.iter().map(|k| k.0 + k.2).fold(0.0, f32::max);
                    match &s.columns {
                        Tracks::List(l) if l.iter().all(|t| matches!(t, Len::Px(_))) => {
                            let w = l.iter().map(|t| t.px(None).unwrap_or(0.0)).sum::<f32>() + g;
                            (w, w)
                        }
                        Tracks::Fill(_) => (min, widest),
                        _ => (min * n + g, widest * n + g),
                    }
                }
                _ => (
                    kids.iter().map(|k| k.0 + k.2).fold(0.0, f32::max),
                    kids.iter().map(|k| k.1 + k.2).fold(0.0, f32::max),
                ),
            }
        };
        let lo = s.min_width.px(None).unwrap_or(0.0);
        let hi = s.max_width.px(None).unwrap_or(INF);
        (clamp(min + extra, lo, hi), clamp(max + extra, lo, hi))
    }

    fn run_intrinsic(&mut self, items: &[Item]) -> (f32, f32) {
        let pieces = self.pieces(items, INF);
        let (mut min, mut max, mut line) = (0.0f32, 0.0f32, 0.0f32);
        let mut word = 0.0f32;
        for p in &pieces {
            match p {
                Piece::Text { w, space, wrap, .. } => {
                    line += w;
                    if *space && *wrap {
                        min = min.max(word);
                        word = 0.0;
                    } else {
                        word += w;
                    }
                }
                Piece::Atom { w, margin, .. } => {
                    let full = w + margin[1] + margin[3];
                    line += full;
                    min = min.max(word).max(full);
                    word = 0.0;
                }
                Piece::Break => {
                    max = max.max(line);
                    min = min.max(word);
                    line = 0.0;
                    word = 0.0;
                }
            }
        }
        (min.max(word), max.max(line))
    }

    /// Words, spaces and atomic inline boxes of an inline run.
    fn pieces(&mut self, items: &[Item], avail: f32) -> Vec<Piece> {
        let mut out = Vec::new();
        for &it in items {
            self.collect_pieces(it, avail, &mut out);
        }
        out
    }

    fn collect_pieces(&mut self, it: Item, avail: f32, out: &mut Vec<Piece>) {
        match it {
            Item::Text(id) => {
                let s = self.s(id).clone();
                let content = text::transform(&s, &self.t.n(id).text);
                let wrap = text::wraps(s.white_space);
                for tok in text::tokens(s.white_space, &content) {
                    match tok {
                        Token::Word(w) => {
                            let width = self.fonts.width(self.m, &s, w);
                            out.push(Piece::Text {
                                node: id,
                                s: w.to_string(),
                                w: width,
                                space: false,
                                wrap,
                            });
                        }
                        Token::Space(sp) => {
                            let width = self.fonts.width(self.m, &s, sp);
                            out.push(Piece::Text {
                                node: id,
                                s: sp.to_string(),
                                w: width,
                                space: true,
                                wrap,
                            });
                        }
                        Token::Break => out.push(Piece::Break),
                    }
                }
            }
            Item::Elem(id) => {
                if self.s(id).display == Display::Inline && !self.replaced(id) {
                    let mut kids = Vec::new();
                    self.items(id, &mut kids);
                    for k in kids {
                        self.collect_pieces(k, avail, out);
                    }
                } else {
                    let (margin, _) = self.margins(id, avail.min(1e6));
                    let a = if avail.is_finite() {
                        avail - margin[1] - margin[3]
                    } else {
                        INF
                    };
                    let w = if a.is_finite() {
                        self.fit_width(id, a)
                    } else {
                        self.intrinsic(id).1
                    };
                    out.push(Piece::Atom { id, w, margin });
                }
            }
        }
    }

    /// Lays out an inline run at `width`; returns its height and text fragments (offset by
    /// `x0`, `y0`). Atomic inline boxes are positioned in place.
    fn inline_run(
        &mut self,
        items: &[Item],
        x0: f32,
        y0: f32,
        width: f32,
        align: TextAlign,
        cb_h: Option<f32>,
    ) -> (f32, Vec<Frag>) {
        let pieces = self.pieces(items, width);
        let mut lines: Vec<Vec<usize>> = vec![Vec::new()];
        let mut x = 0.0;
        for (i, p) in pieces.iter().enumerate() {
            match p {
                Piece::Break => {
                    lines.push(Vec::new());
                    x = 0.0;
                }
                Piece::Text {
                    w,
                    space: true,
                    wrap,
                    ..
                } => {
                    let line = lines.last_mut().unwrap();
                    if line.is_empty() && *wrap {
                        continue;
                    }
                    line.push(i);
                    x += w;
                }
                _ => {
                    let (w, wrap) = match p {
                        Piece::Text { w, wrap, .. } => (*w, *wrap),
                        Piece::Atom { w, margin, .. } => (w + margin[1] + margin[3], true),
                        Piece::Break => unreachable!(),
                    };
                    let line = lines.last_mut().unwrap();
                    let has_content = line
                        .iter()
                        .any(|&j| !matches!(pieces[j], Piece::Text { space: true, .. }));
                    if wrap && has_content && x + w > width + 0.01 {
                        while let Some(&j) = line.last() {
                            if matches!(pieces[j], Piece::Text { space: true, .. }) {
                                line.pop();
                            } else {
                                break;
                            }
                        }
                        lines.push(vec![i]);
                        x = w;
                    } else {
                        line.push(i);
                        x += w;
                    }
                }
            }
        }
        let mut frags: Vec<Frag> = Vec::new();
        let mut y = 0.0;
        let n_lines = lines.len();
        for (li, line) in lines.into_iter().enumerate() {
            // Trailing collapsible spaces do not count.
            let mut end = line.len();
            while end > 0
                && matches!(
                    pieces[line[end - 1]],
                    Piece::Text {
                        space: true,
                        wrap: true,
                        ..
                    }
                )
            {
                end -= 1;
            }
            let line = &line[..end];
            if line.is_empty() {
                if n_lines > 1 && li < n_lines - 1 {
                    // An empty line from a forced break still takes a line of height.
                    y += items
                        .first()
                        .map(|&it| self.item_line_height(it))
                        .unwrap_or(0.0);
                }
                continue;
            }
            let mut heights = Vec::with_capacity(line.len());
            let mut line_w = 0.0;
            for &i in line {
                let (w, h) = match &pieces[i] {
                    Piece::Text { node, w, .. } => (*w, self.s(*node).line_height),
                    Piece::Atom { id, w, margin } => {
                        let fixed = self.fixed_h(*id, cb_h);
                        let h = self.layout(*id, *w, fixed);
                        (w + margin[1] + margin[3], h + margin[0] + margin[2])
                    }
                    Piece::Break => (0.0, 0.0),
                };
                heights.push(h);
                line_w += w;
            }
            let line_h = heights.iter().copied().fold(0.0, f32::max);
            let mut x = match align {
                TextAlign::Left => 0.0,
                TextAlign::Center => ((width - line_w) / 2.0).max(0.0),
                TextAlign::Right => (width - line_w).max(0.0),
            };
            for (k, &i) in line.iter().enumerate() {
                let h = heights[k];
                let top = y + (line_h - h) / 2.0;
                match &pieces[i] {
                    Piece::Text { node, s, w, .. } => {
                        match frags.last_mut() {
                            Some(f)
                                if f.node == *node
                                    && (f.rect.y - (y0 + top)).abs() < 0.01
                                    && (f.rect.x + f.rect.w - (x0 + x)).abs() < 0.01 =>
                            {
                                f.text.push_str(s);
                                f.rect.w += w;
                            }
                            _ => frags.push(Frag {
                                node: *node,
                                rect: Rect {
                                    x: x0 + x,
                                    y: y0 + top,
                                    w: *w,
                                    h,
                                },
                                text: s.clone(),
                            }),
                        }
                        x += w;
                    }
                    Piece::Atom { id, w, margin } => {
                        let r = &mut self.t.m(*id).lay.rect;
                        *r = Rect {
                            x: x0 + x + margin[3],
                            y: y0 + top + margin[0],
                            w: *w,
                            h: h - margin[0] - margin[2],
                        };
                        x += w + margin[1] + margin[3];
                    }
                    Piece::Break => {}
                }
            }
            y += line_h;
        }
        (y, frags)
    }

    fn item_line_height(&self, it: Item) -> f32 {
        match it {
            Item::Text(id) | Item::Elem(id) => self.s(id).line_height,
        }
    }

    /// The border-box height `layout` would produce, without needing child positions.
    fn measure(&mut self, id: Id, w: f32, h: Option<f32>) -> f32 {
        let lay = &self.t.n(id).lay;
        if let Some((cw, ch, out)) = lay.cache
            && cw == w
            && ch == h
        {
            return out;
        }
        if let Some(&(_, _, out)) = lay.measures.iter().find(|m| m.0 == w && m.1 == h) {
            return out;
        }
        let out = self.layout(id, w, h);
        let m = &mut self.t.m(id).lay.measures;
        if m.len() >= 4 {
            m.remove(0);
        }
        m.push((w, h, out));
        out
    }

    /// Lays out `id` with border-box width `w` and, if definite, height `h`. Returns the
    /// border-box height and stores child positions.
    pub fn layout(&mut self, id: Id, w: f32, h: Option<f32>) -> f32 {
        if let Some((cw, ch, out)) = self.t.n(id).lay.cache
            && cw == w
            && ch == h
        {
            return out;
        }
        let (p, b) = self.pb(id, w);
        let cx0 = b[3] + p[3];
        let cy0 = b[0] + p[0];
        let content_w = (w - p[1] - p[3] - b[1] - b[3]).max(0.0);
        let vert = p[0] + p[2] + b[0] + b[2];
        let content_h_def = h.map(|h| (h - vert).max(0.0));
        let s = self.s(id).clone();
        self.t.m(id).lay.frags.clear();
        let content_h = if self.replaced(id) {
            self.replaced_size(id, Some(content_w)).1
        } else {
            match s.display {
                Display::Flex | Display::InlineFlex => {
                    self.flex(id, &s, cx0, cy0, content_w, content_h_def)
                }
                Display::Grid => self.grid(id, &s, cx0, cy0, content_w, content_h_def),
                _ => self.flow(id, &s, cx0, cy0, content_w, content_h_def),
            }
        };
        let out = match h {
            Some(h) => h,
            None => self.clamp_h(id, content_h + vert, None),
        };
        // Buttons center their content vertically.
        if self.t.n(id).is("button") && out - vert > content_h + 0.5 {
            let dy = (out - vert - content_h) / 2.0;
            self.shift_content(id, dy);
        }
        self.position_out_of_flow(id, w, out);
        let lay = &mut self.t.m(id).lay;
        lay.padding = p;
        lay.border = b;
        lay.cache = Some((w, h, out));
        if s.clips() {
            self.measure_scroll(id, w, out);
        }
        self.extent(id, &s, w, out);
        out
    }

    fn extent(&mut self, id: Id, s: &Style, w: f32, h: f32) {
        let mut e = Rect {
            x: 0.0,
            y: 0.0,
            w,
            h,
        };
        let union = |e: &mut Rect, r: Rect| {
            let (x1, y1) = ((e.x + e.w).max(r.x + r.w), (e.y + e.h).max(r.y + r.h));
            e.x = e.x.min(r.x);
            e.y = e.y.min(r.y);
            e.w = x1 - e.x;
            e.h = y1 - e.y;
        };
        if !s.clips() {
            let n = self.t.n(id);
            for f in &n.lay.frags {
                union(&mut e, f.rect);
            }
            for &c in &n.children {
                let cl = &self.t.n(c).lay;
                if self.t.n(c).kind == Kind::Element && self.t.n(c).style.display != Display::None {
                    let r = Rect {
                        x: cl.rect.x + cl.offset.0 + cl.extent.x,
                        y: cl.rect.y + cl.offset.1 + cl.extent.y,
                        w: cl.extent.w,
                        h: cl.extent.h,
                    };
                    union(&mut e, r);
                }
            }
        }
        for sh in &s.shadow {
            let grow = sh.blur + sh.spread.max(0.0);
            union(
                &mut e,
                Rect {
                    x: sh.x - grow,
                    y: sh.y - grow,
                    w: w + 2.0 * grow,
                    h: h + 2.0 * grow,
                },
            );
        }
        if !s.transform.is_empty() {
            // Transformed boxes may paint anywhere; never cull them.
            e = Rect {
                x: -1e9,
                y: -1e9,
                w: 2e9,
                h: 2e9,
            };
        }
        self.t.m(id).lay.extent = e;
    }

    fn shift_content(&mut self, id: Id, dy: f32) {
        for f in &mut self.t.m(id).lay.frags {
            f.rect.y += dy;
        }
        let mut items = Vec::new();
        self.items(id, &mut items);
        for it in items {
            if let Item::Elem(c) = it {
                self.t.m(c).lay.rect.y += dy;
            }
        }
    }

    fn measure_scroll(&mut self, id: Id, w: f32, h: f32) {
        let mut items = Vec::new();
        self.items(id, &mut items);
        let lay = &self.t.n(id).lay;
        let (p, b) = (lay.padding, lay.border);
        let (mut cw, mut ch) = (w - b[1] - b[3], h - b[0] - b[2]);
        for f in &lay.frags {
            cw = cw.max(f.rect.x + f.rect.w + p[1] - b[3]);
            ch = ch.max(f.rect.y + f.rect.h + p[2] - b[0]);
        }
        for it in items {
            if let Item::Elem(c) = it {
                let r = self.t.n(c).lay.rect;
                let (m, _) = self.margins(c, w);
                cw = cw.max(r.x + r.w + m[1] + p[1] - b[3]);
                ch = ch.max(r.y + r.h + m[2] + p[2] - b[0]);
            }
        }
        let lay = &mut self.t.m(id).lay;
        lay.content = (cw, ch);
        let max = lay.max_scroll();
        lay.scroll = (
            clamp(lay.scroll.0, 0.0, max.0),
            clamp(lay.scroll.1, 0.0, max.1),
        );
    }

    /// Normal flow: block children stacked vertically, inline content in line boxes.
    fn flow(&mut self, id: Id, s: &Style, x0: f32, y0: f32, w: f32, h: Option<f32>) -> f32 {
        let mut items = Vec::new();
        self.items(id, &mut items);
        let mut y = 0.0;
        let mut run: Vec<Item> = Vec::new();
        let mut frags = Vec::new();
        let flush = |cx: &mut Self, run: &mut Vec<Item>, y: &mut f32, frags: &mut Vec<Frag>| {
            if run.is_empty() {
                return;
            }
            let (rh, f) = cx.inline_run(run, x0, y0 + *y, w, s.text_align, h);
            *y += rh;
            frags.extend(f);
            run.clear();
        };
        for it in items {
            if self.inline_level(it) {
                run.push(it);
                continue;
            }
            flush(self, &mut run, &mut y, &mut frags);
            let Item::Elem(c) = it else { unreachable!() };
            let (m, auto) = self.margins(c, w);
            let avail = (w - m[1] - m[3]).max(0.0);
            let cw = match self.explicit_width(c, w) {
                Some(cw) => cw,
                None => self.clamp_w(c, avail, w),
            };
            let ch = self.layout(c, cw, self.fixed_h(c, h));
            let free = avail - cw;
            let x = match (auto[3], auto[1]) {
                (true, true) => m[3] + free / 2.0,
                (true, false) => m[3] + free,
                _ => m[3],
            };
            y += m[0];
            let r = &mut self.t.m(c).lay.rect;
            r.x = x0 + x;
            r.y = y0 + y;
            r.w = cw;
            r.h = ch;
            y += ch + m[2];
        }
        flush(self, &mut run, &mut y, &mut frags);
        self.t.m(id).lay.frags.extend(frags);
        self.relative_offsets(id, w, h);
        y
    }

    fn relative_offsets(&mut self, id: Id, cb_w: f32, cb_h: Option<f32>) {
        let mut items = Vec::new();
        self.items(id, &mut items);
        for it in items {
            if let Item::Elem(c) = it {
                let s = self.s(c);
                if s.position == Position::Relative {
                    let [t, r, b, l] = s.inset;
                    let dx = l
                        .px(Some(cb_w))
                        .or(r.px(Some(cb_w)).map(|v| -v))
                        .unwrap_or(0.0);
                    let dy = t.px(cb_h).or(b.px(cb_h).map(|v| -v)).unwrap_or(0.0);
                    self.t.m(c).lay.offset = (dx, dy);
                } else {
                    self.t.m(c).lay.offset = (0.0, 0.0);
                }
            }
        }
    }

    /// Positions absolutely positioned children against this element's padding box and
    /// fixed ones against the viewport.
    fn position_out_of_flow(&mut self, id: Id, w: f32, h: f32) {
        let mut abs = Vec::new();
        self.out_of_flow(id, &mut abs);
        let (_, b) = self.pb(id, w);
        for c in abs {
            let fixed = self.s(c).position == Position::Fixed;
            let (ox, oy, cw, ch) = if fixed {
                (0.0, 0.0, self.vw, self.vh)
            } else {
                (b[3], b[0], w - b[1] - b[3], h - b[0] - b[2])
            };
            let [t, r, bo, l] = self.s(c).inset.map(|v| v);
            let (m, _) = self.margins(c, cw);
            let (l, r) = (l.px(Some(cw)), r.px(Some(cw)));
            let (t, bo) = (t.px(Some(ch)), bo.px(Some(ch)));
            let width = match (self.explicit_width(c, cw), l, r) {
                (Some(x), ..) => x,
                (None, Some(l), Some(r)) => (cw - l - r - m[1] - m[3]).max(0.0),
                _ => self.fit_width(
                    c,
                    (cw - l.unwrap_or(0.0) - r.unwrap_or(0.0) - m[1] - m[3]).max(0.0),
                ),
            };
            let fixed_h = match (self.fixed_h(c, Some(ch)), t, bo) {
                (Some(x), ..) => Some(x),
                (None, Some(t), Some(bo)) => Some((ch - t - bo - m[0] - m[2]).max(0.0)),
                _ => None,
            };
            let height = self.layout(c, width, fixed_h);
            let x = match (l, r) {
                (Some(l), _) => ox + l + m[3],
                (None, Some(r)) => ox + cw - r - width - m[1],
                _ => ox + self.t.n(id).lay.padding[3] + m[3],
            };
            let y = match (t, bo) {
                (Some(t), _) => oy + t + m[0],
                (None, Some(bo)) => oy + ch - bo - height - m[2],
                _ => oy + self.t.n(id).lay.padding[0] + m[0],
            };
            let lay = &mut self.t.m(c).lay;
            lay.rect = Rect {
                x,
                y,
                w: width,
                h: height,
            };
            lay.offset = (0.0, 0.0);
        }
    }

    /// Measures an item for flexbox: width when `row`, otherwise height at width `cross`.
    fn item_content_main(&mut self, it: Item, row: bool, cross: f32, cb_h: Option<f32>) -> f32 {
        match it {
            Item::Text(_) => {
                if row {
                    self.run_intrinsic(&[it]).1
                } else {
                    self.inline_run(&[it], 0.0, 0.0, cross, TextAlign::Left, cb_h)
                        .0
                }
            }
            Item::Elem(c) => {
                if row {
                    self.intrinsic(c).1
                } else {
                    let fixed = self.fixed_h(c, cb_h);
                    self.measure(c, cross, fixed)
                }
            }
        }
    }

    fn flex(&mut self, id: Id, s: &Style, x0: f32, y0: f32, cw: f32, ch: Option<f32>) -> f32 {
        let mut items = Vec::new();
        self.items(id, &mut items);
        let row = s.row;
        let (gap_main, gap_cross) = if row {
            (s.col_gap, s.row_gap)
        } else {
            (s.row_gap, s.col_gap)
        };
        let avail_main = if row { Some(cw) } else { ch };
        struct F {
            it: Item,
            m: [f32; 4],
            auto: [bool; 4],
            base: f32,
            min: f32,
            max: f32,
            size: f32,
            cross: f32,
            cross_w: f32,
            align: Align,
            grow: f32,
            shrink: f32,
        }
        let mut fs: Vec<F> = Vec::with_capacity(items.len());
        for it in items {
            let (m, auto, st) = match it {
                Item::Elem(c) => {
                    let (m, a) = self.margins(c, cw);
                    (m, a, Some(self.s(c).clone()))
                }
                Item::Text(_) => ([0.0; 4], [false; 4], None),
            };
            let align = st.as_ref().and_then(|x| x.align_self).unwrap_or(s.align);
            // Cross width for column items.
            let cross_w = if row {
                0.0
            } else {
                match it {
                    Item::Elem(c) => match self.explicit_width(c, cw) {
                        Some(w) => w,
                        None if align == Align::Stretch => {
                            self.clamp_w(c, (cw - m[1] - m[3]).max(0.0), cw)
                        }
                        None => self.fit_width(c, (cw - m[1] - m[3]).max(0.0)),
                    },
                    Item::Text(_) => cw,
                }
            };
            let (base, min, max, grow, shrink) = match (&st, it) {
                (Some(st), Item::Elem(c)) => {
                    let main_prop = if row { st.width } else { st.height };
                    let main_cb = if row { Some(cw) } else { ch };
                    let base = match st.basis.px(avail_main).or(main_prop.px(main_cb)) {
                        Some(b) => b,
                        None => self.item_content_main(it, row, cross_w, ch),
                    };
                    let (min_p, max_p) = if row {
                        (st.min_width, st.max_width)
                    } else {
                        (st.min_height, st.max_height)
                    };
                    let min = match min_p.px(main_cb) {
                        Some(v) => v,
                        None if st.clips() => 0.0,
                        None if row => {
                            let mc = self.intrinsic_of(c, false).0;
                            main_prop.px(main_cb).map_or(mc, |w| w.min(mc))
                        }
                        None => {
                            let content = self.item_content_main(it, false, cross_w, ch);
                            main_prop.px(main_cb).map_or(content, |h| h.min(content))
                        }
                    };
                    let max = max_p.px(main_cb).unwrap_or(INF);
                    (base, min, max, st.grow, st.shrink)
                }
                _ => {
                    let base = self.item_content_main(it, row, cross_w, ch);
                    let min = if row {
                        self.run_intrinsic(&[it]).0
                    } else {
                        base
                    };
                    (base, min, INF, 0.0, 1.0)
                }
            };
            let hyp = clamp(base, min, max);
            fs.push(F {
                it,
                m,
                auto,
                base,
                min,
                max,
                size: hyp,
                cross: 0.0,
                cross_w,
                align,
                grow,
                shrink,
            });
        }
        let main_margin = |f: &F| {
            if row {
                f.m[1] + f.m[3]
            } else {
                f.m[0] + f.m[2]
            }
        };
        // Break into lines.
        let mut lines: Vec<std::ops::Range<usize>> = Vec::new();
        let mut start = 0;
        if s.wrap
            && let Some(avail) = avail_main
        {
            let mut used = 0.0;
            for (i, f) in fs.iter().enumerate() {
                let w = f.size + main_margin(f);
                if i > start && used + gap_main + w > avail + 0.01 {
                    lines.push(start..i);
                    start = i;
                    used = w;
                } else {
                    used += if i > start { gap_main + w } else { w };
                }
            }
        }
        lines.push(start..fs.len());
        // Resolve flexible lengths.
        if let Some(avail) = avail_main {
            for line in &lines {
                let fl = &mut fs[line.clone()];
                let gaps = gap_main * fl.len().saturating_sub(1) as f32;
                let margins: f32 = fl.iter().map(main_margin).sum();
                let hyp_sum: f32 = fl.iter().map(|f| f.size).sum();
                let growing = avail - hyp_sum - margins - gaps > 0.0;
                let mut frozen: Vec<bool> = fl
                    .iter()
                    .map(|f| {
                        if growing {
                            f.grow == 0.0
                        } else {
                            f.shrink == 0.0
                        }
                    })
                    .collect();
                for _ in 0..=fl.len() {
                    if frozen.iter().all(|&x| x) {
                        break;
                    }
                    let used: f32 = fl
                        .iter()
                        .zip(&frozen)
                        .map(|(f, &fr)| if fr { f.size } else { f.base })
                        .sum();
                    let free = avail - used - margins - gaps;
                    let weight = |f: &F| if growing { f.grow } else { f.shrink * f.base };
                    let total: f32 = fl
                        .iter()
                        .zip(&frozen)
                        .filter(|(_, fr)| !**fr)
                        .map(|(f, _)| weight(f))
                        .sum();
                    let mut violation = 0.0;
                    let mut clamped = vec![0i8; fl.len()];
                    for (i, f) in fl.iter_mut().enumerate() {
                        if frozen[i] {
                            continue;
                        }
                        let share = if total > 0.0 {
                            free * weight(f) / total
                        } else {
                            0.0
                        };
                        let target = f.base + share;
                        let c = clamp(target, f.min, f.max);
                        violation += c - target;
                        clamped[i] = if c > target {
                            1
                        } else if c < target {
                            -1
                        } else {
                            0
                        };
                        f.size = c;
                    }
                    if violation.abs() < 0.01 {
                        break;
                    }
                    for i in 0..fl.len() {
                        if !frozen[i]
                            && ((violation > 0.0 && clamped[i] > 0)
                                || (violation < 0.0 && clamped[i] < 0))
                        {
                            frozen[i] = true;
                        }
                    }
                }
            }
        }
        // Cross sizes.
        let mut line_cross: Vec<f32> = Vec::with_capacity(lines.len());
        for line in &lines {
            let mut lc: f32 = 0.0;
            for f in &mut fs[line.clone()] {
                f.cross = match f.it {
                    Item::Elem(c) if row => {
                        let fixed = self.fixed_h(c, ch);
                        self.layout(c, f.size, fixed)
                    }
                    Item::Elem(c) => {
                        self.layout(c, f.cross_w, Some(f.size));
                        f.cross_w
                    }
                    Item::Text(_) if row => {
                        self.inline_run(&[f.it], 0.0, 0.0, f.size, TextAlign::Left, ch)
                            .0
                    }
                    Item::Text(_) => f.cross_w,
                };
                let cm = if row {
                    f.m[0] + f.m[2]
                } else {
                    f.m[1] + f.m[3]
                };
                lc = lc.max(f.cross + cm);
            }
            line_cross.push(lc);
        }
        let container_cross = if row { ch } else { Some(cw) };
        if lines.len() == 1
            && let Some(cc) = container_cross
        {
            line_cross[0] = cc;
        }
        // Stretch.
        for (li, line) in lines.iter().enumerate() {
            for f in &mut fs[line.clone()] {
                let Item::Elem(c) = f.it else { continue };
                if row
                    && f.align == Align::Stretch
                    && self.s(c).height == Len::Auto
                    && !f.auto[0]
                    && !f.auto[2]
                {
                    let target = self.clamp_h(c, (line_cross[li] - f.m[0] - f.m[2]).max(0.0), ch);
                    if (target - f.cross).abs() > 0.01 {
                        self.layout(c, f.size, Some(target));
                        f.cross = target;
                    }
                }
            }
        }
        // Positions.
        let mut cross_pos = 0.0;
        let mut total_main: f32 = 0.0;
        for (li, line) in lines.iter().enumerate() {
            let n = line.len();
            let gaps = gap_main * n.saturating_sub(1) as f32;
            let used: f32 = fs[line.clone()]
                .iter()
                .map(|f| f.size + main_margin(f))
                .sum::<f32>()
                + gaps;
            total_main = total_main.max(used);
            let free = avail_main.map_or(0.0, |a| a - used);
            let auto_margins: usize = fs[line.clone()]
                .iter()
                .map(|f| {
                    if row {
                        f.auto[1] as usize + f.auto[3] as usize
                    } else {
                        f.auto[0] as usize + f.auto[2] as usize
                    }
                })
                .sum();
            let (mut pos, between) = if auto_margins > 0 || free <= 0.0 {
                (0.0, 0.0)
            } else {
                match s.justify {
                    Justify::Start => (0.0, 0.0),
                    Justify::Center => (free / 2.0, 0.0),
                    Justify::End => (free, 0.0),
                    Justify::Between if n > 1 => (0.0, free / (n - 1) as f32),
                    Justify::Between => (0.0, 0.0),
                    Justify::Around => (free / n as f32 / 2.0, free / n as f32),
                    Justify::Evenly => (free / (n + 1) as f32, free / (n + 1) as f32),
                }
            };
            let auto_share = if auto_margins > 0 && free > 0.0 {
                free / auto_margins as f32
            } else {
                0.0
            };
            let order: Vec<usize> = if s.reverse {
                line.clone().rev().collect()
            } else {
                line.clone().collect()
            };
            let main_extent = avail_main.unwrap_or(used);
            for (k, i) in order.into_iter().enumerate() {
                let f = &fs[i];
                let (m_start, m_end, a_start, a_end) = if row {
                    (f.m[3], f.m[1], f.auto[3], f.auto[1])
                } else {
                    (f.m[0], f.m[2], f.auto[0], f.auto[2])
                };
                pos += m_start + if a_start { auto_share } else { 0.0 };
                let main_at = if s.reverse && avail_main.is_some() {
                    main_extent - pos - f.size
                } else {
                    pos
                };
                pos += f.size + m_end + if a_end { auto_share } else { 0.0 };
                if k + 1 < n {
                    pos += gap_main + between;
                }
                let (c_start, c_end) = if row {
                    (f.m[0], f.m[2])
                } else {
                    (f.m[3], f.m[1])
                };
                let room = line_cross[li] - f.cross - c_start - c_end;
                let cross_at = cross_pos
                    + c_start
                    + match f.align {
                        Align::Start | Align::Stretch => 0.0,
                        Align::Center => room / 2.0,
                        Align::End => room,
                    };
                let (x, y, w, h) = if row {
                    (main_at, cross_at, f.size, f.cross)
                } else {
                    (cross_at, main_at, f.cross, f.size)
                };
                match f.it {
                    Item::Elem(c) => {
                        self.t.m(c).lay.rect = Rect {
                            x: x0 + x,
                            y: y0 + y,
                            w,
                            h,
                        }
                    }
                    Item::Text(_) => {
                        let (_, frags) =
                            self.inline_run(&[f.it], x0 + x, y0 + y, w, s.text_align, ch);
                        self.t.m(id).lay.frags.extend(frags);
                    }
                }
            }
            cross_pos += line_cross[li] + gap_cross;
        }
        self.relative_offsets(id, cw, ch);
        let cross_total =
            line_cross.iter().sum::<f32>() + gap_cross * lines.len().saturating_sub(1) as f32;
        if row { cross_total } else { total_main }
    }

    fn grid(&mut self, id: Id, s: &Style, x0: f32, y0: f32, cw: f32, ch: Option<f32>) -> f32 {
        let mut items = Vec::new();
        self.items(id, &mut items);
        let g = s.col_gap;
        let n = if s.stack {
            1
        } else {
            match &s.columns {
                Tracks::Count(n) => (*n).max(1) as usize,
                Tracks::Fill(min) => {
                    (((cw + g) / (min.min(cw).max(1.0) + g)).floor() as usize).max(1)
                }
                Tracks::List(l) if !l.is_empty() => l.len(),
                _ => 1,
            }
        };
        // Auto placement, row by row.
        let mut cells: Vec<(usize, usize, usize)> = Vec::with_capacity(items.len());
        let (mut r, mut c) = (0, 0);
        for it in &items {
            if s.stack {
                cells.push((0, 0, 1));
                continue;
            }
            let span = match it {
                Item::Elem(e) => (self.s(*e).col_span as usize).clamp(1, n),
                Item::Text(_) => 1,
            };
            if c + span > n {
                r += 1;
                c = 0;
            }
            cells.push((r, c, span));
            c += span;
            if c >= n {
                r += 1;
                c = 0;
            }
        }
        // Column widths.
        let avail = (cw - g * (n - 1) as f32).max(0.0);
        let widths: Vec<f32> = match &s.columns {
            Tracks::List(l) if !s.stack && !l.is_empty() => {
                let mut w: Vec<f32> = vec![0.0; n];
                let mut fr_total = 0.0;
                for (i, t) in l.iter().enumerate() {
                    match t {
                        Len::Fr(f) => fr_total += f,
                        Len::Auto
                        | Len::MinContent
                        | Len::MaxContent
                        | Len::FitContent
                        | Len::None => {
                            w[i] = items
                                .iter()
                                .zip(&cells)
                                .filter(|(_, cell)| cell.1 == i && cell.2 == 1)
                                .map(|(it, _)| self.item_max(*it))
                                .fold(0.0, f32::max);
                        }
                        _ => w[i] = t.px(Some(cw)).unwrap_or(0.0),
                    }
                }
                let rest = (avail - w.iter().sum::<f32>()).max(0.0);
                for (i, t) in l.iter().enumerate() {
                    if let Len::Fr(f) = t {
                        w[i] = rest * f / fr_total;
                    }
                }
                w
            }
            _ => vec![avail / n as f32; n],
        };
        let col_x: Vec<f32> = widths
            .iter()
            .scan(0.0, |x, w| {
                let at = *x;
                *x += w + g;
                Some(at)
            })
            .collect();
        let rows = cells.iter().map(|c| c.0 + 1).max().unwrap_or(0);
        let fixed_rows: Option<Vec<Len>> = match &s.rows {
            Tracks::List(l) => Some(l.clone()),
            Tracks::Count(k) => Some(vec![Len::Fr(1.0); *k as usize]),
            _ => None,
        };
        let mut heights = vec![0.0f32; rows];
        let mut sizes: Vec<(f32, f32, [f32; 4])> = Vec::with_capacity(items.len());
        for (it, &(r, c, span)) in items.iter().zip(&cells) {
            let cell_w = widths[c..c + span].iter().sum::<f32>() + g * (span - 1) as f32;
            let (w, h, m) = match *it {
                Item::Elem(e) => {
                    let (m, _) = self.margins(e, cw);
                    let w = self.explicit_width(e, cell_w).unwrap_or_else(|| {
                        self.clamp_w(e, (cell_w - m[1] - m[3]).max(0.0), cell_w)
                    });
                    let fixed = self.fixed_h(e, None);
                    (w, self.layout(e, w, fixed), m)
                }
                Item::Text(_) => (
                    cell_w,
                    self.inline_run(&[*it], 0.0, 0.0, cell_w, s.text_align, ch)
                        .0,
                    [0.0; 4],
                ),
            };
            heights[r] = heights[r].max(h + m[0] + m[2]);
            sizes.push((w, h, m));
        }
        let rg = s.row_gap;
        if let Some(tracks) = &fixed_rows {
            let gaps = rg * rows.saturating_sub(1) as f32;
            let mut fr_total = 0.0;
            for (i, h) in heights.iter_mut().enumerate() {
                match tracks.get(i) {
                    Some(Len::Fr(f)) => fr_total += f,
                    Some(t) => {
                        if let Some(v) = t.px(ch) {
                            *h = v;
                        }
                    }
                    None => {}
                }
            }
            if let Some(chv) = ch {
                let fixed: f32 = heights
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !matches!(tracks.get(*i), Some(Len::Fr(_))))
                    .map(|(_, h)| h)
                    .sum();
                let rest = (chv - fixed - gaps).max(0.0);
                for (i, h) in heights.iter_mut().enumerate() {
                    if let Some(Len::Fr(f)) = tracks.get(i) {
                        *h = h.max(rest * f / fr_total);
                    }
                }
            }
        } else if s.stack
            && let Some(chv) = ch
        {
            heights.iter_mut().for_each(|h| *h = h.max(chv));
        }
        let row_y: Vec<f32> = heights
            .iter()
            .scan(0.0, |y, h| {
                let at = *y;
                *y += h + rg;
                Some(at)
            })
            .collect();
        for (k, (it, &(r, c, _))) in items.iter().zip(&cells).enumerate() {
            let (w, mut h, m) = sizes[k];
            let x = col_x[c] + m[3];
            let room = heights[r] - h - m[0] - m[2];
            let mut y = row_y[r] + m[0];
            match *it {
                Item::Elem(e) => {
                    let align = self.s(e).align_self.unwrap_or(s.align);
                    match align {
                        Align::Stretch if self.s(e).height == Len::Auto && room > 0.01 => {
                            h = heights[r] - m[0] - m[2];
                            self.layout(e, w, Some(h));
                        }
                        Align::Center => y += room / 2.0,
                        Align::End => y += room,
                        _ => {}
                    }
                    self.t.m(e).lay.rect = Rect {
                        x: x0 + x,
                        y: y0 + y,
                        w,
                        h,
                    };
                }
                Item::Text(_) => {
                    let (_, frags) = self.inline_run(&[*it], x0 + x, y0 + y, w, s.text_align, ch);
                    self.t.m(id).lay.frags.extend(frags);
                }
            }
        }
        self.relative_offsets(id, cw, ch);
        heights.iter().sum::<f32>() + rg * rows.saturating_sub(1) as f32
    }

    fn item_max(&mut self, it: Item) -> f32 {
        match it {
            Item::Elem(e) => {
                let (m, _) = self.margins(e, 0.0);
                self.intrinsic(e).1 + m[1] + m[3]
            }
            Item::Text(_) => self.run_intrinsic(&[it]).1,
        }
    }
}

/// Wraps field text into lines of at most `width`; returns byte ranges. Used for `textarea`.
pub fn wrap_lines(
    fonts: &mut Fonts,
    m: &mut dyn Measure,
    s: &Style,
    text: &str,
    width: f32,
) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let base = para.as_ptr() as usize - text.as_ptr() as usize;
        let mut start = 0;
        let mut last_break = None;
        let mut i = 0;
        let chars: Vec<(usize, char)> = para.char_indices().collect();
        while i < chars.len() {
            let (at, c) = chars[i];
            let end = at + c.len_utf8();
            if c == ' ' {
                last_break = Some(end);
            }
            if fonts.width(m, s, &para[start..end]) > width && end - start > c.len_utf8() {
                let cut = last_break.filter(|&b| b > start).unwrap_or(at);
                out.push((base + start, base + cut));
                start = cut;
                last_break = None;
                i = chars
                    .iter()
                    .position(|&(a, _)| a >= cut)
                    .unwrap_or(chars.len());
                continue;
            }
            i += 1;
        }
        out.push((base + start, base + para.len()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::tests::Buf;
    use crate::style::{Media, Sheet};
    use crate::text::tests::Mono;

    struct World {
        t: Tree,
        fonts: Fonts,
        images: HashMap<String, (f32, f32)>,
    }

    fn world(css: &str, b: &Buf, vw: f32) -> World {
        let mut t = Tree::default();
        t.apply(&b.0, &mut Vec::new()).unwrap();
        let media = Media {
            width: vw,
            height: 600.0,
            dark: false,
            reduced_motion: false,
        };
        Sheet::new(css).restyle(&mut t, &media, true);
        let mut w = World {
            t,
            fonts: Fonts::default(),
            images: HashMap::new(),
        };
        w.run(vw);
        w
    }

    impl World {
        fn run(&mut self, vw: f32) {
            let mut m = Mono;
            let mut cx = Cx {
                t: &mut self.t,
                m: &mut m,
                fonts: &mut self.fonts,
                images: &self.images,
                vw,
                vh: 600.0,
            };
            layout_root(&mut cx);
        }
        fn r(&self, id: Id) -> Rect {
            self.t.n(id).lay.rect
        }
    }

    #[test]
    fn block_flow_and_text_wrap() {
        let mut b = Buf::default();
        b.create(1, "div").class(1, "a").insert(0, 1, 0);
        // 8px per character at 16px.
        b.text(2, "aaaa bbbb cccc").insert(1, 2, 0);
        b.create(3, "div").class(3, "b").insert(1, 3, 0);
        let w = world(
            ".a{width:100px;padding:10px}.b{height:20px;margin:5px auto;width:40px}",
            &b,
            800.0,
        );
        assert_eq!(
            w.r(1),
            Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 10.0 + 48.0 + 5.0 + 20.0 + 5.0 + 10.0
            }
        );
        let frags = &w.t.n(1).lay.frags;
        assert_eq!(frags.len(), 2);
        assert_eq!(frags[0].text, "aaaa bbbb");
        assert_eq!(
            frags[0].rect,
            Rect {
                x: 10.0,
                y: 10.0,
                w: 72.0,
                h: 24.0
            }
        );
        assert_eq!(frags[1].text, "cccc");
        assert_eq!(
            w.r(3),
            Rect {
                x: 30.0,
                y: 63.0,
                w: 40.0,
                h: 20.0
            }
        );
    }

    #[test]
    fn changed_text_is_laid_out_again() {
        let mut b = Buf::default();
        b.create(1, "div").class(1, "v-row").insert(0, 1, 0);
        b.create(3, "div").insert(1, 3, 0);
        b.text(2, "0/100").insert(3, 2, 0);
        let mut w = world(".v-row{display:flex}", &b, 800.0);
        assert_eq!(w.t.n(3).lay.frags[0].text, "0/100");
        let mut b = Buf::default();
        b.int(crate::dom::opc::SET_TEXT).int(2).str("26/100 mid");
        w.t.apply(&b.0, &mut Vec::new()).unwrap();
        w.run(800.0);
        assert_eq!(w.t.n(3).lay.frags[0].text, "26/100 mid");
    }

    #[test]
    fn flex_row_grow_gap_and_align() {
        let mut b = Buf::default();
        b.create(1, "div").class(1, "v-row r").insert(0, 1, 0);
        for (i, c) in [(2, "f"), (3, "g"), (4, "f")] {
            b.create(i, "div").class(i, c).insert(1, i, 0);
        }
        let w = world(
            ".r{width:300px;height:100px;gap:10px;align-items:center}.f{width:50px;height:20px}.g{flex-grow:1;height:40px}",
            &b,
            800.0,
        );
        assert_eq!(
            w.r(2),
            Rect {
                x: 0.0,
                y: 40.0,
                w: 50.0,
                h: 20.0
            }
        );
        assert_eq!(
            w.r(3),
            Rect {
                x: 60.0,
                y: 30.0,
                w: 180.0,
                h: 40.0
            }
        );
        assert_eq!(
            w.r(4),
            Rect {
                x: 250.0,
                y: 40.0,
                w: 50.0,
                h: 20.0
            }
        );
    }

    #[test]
    fn flex_column_stretch_justify_and_text_items() {
        let mut b = Buf::default();
        b.create(1, "div").class(1, "v-col c").insert(0, 1, 0);
        b.text(2, "hello").insert(1, 2, 0);
        b.create(3, "div").class(3, "k").insert(1, 3, 0);
        let w = world(
            ".c{width:200px;height:200px;justify-content:space-between}.k{height:30px}",
            &b,
            800.0,
        );
        assert_eq!(
            w.t.n(1).lay.frags[0].rect,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 40.0,
                h: 24.0
            }
        );
        assert_eq!(
            w.r(3),
            Rect {
                x: 0.0,
                y: 170.0,
                w: 200.0,
                h: 30.0
            }
        );
    }

    #[test]
    fn flex_shrink_respects_min_content() {
        let mut b = Buf::default();
        b.create(1, "div").class(1, "v-row r").insert(0, 1, 0);
        b.create(2, "div")
            .insert(1, 2, 0)
            .text(3, "abcdefghij")
            .insert(2, 3, 0);
        b.create(4, "div").class(4, "big").insert(1, 4, 0);
        let w = world(".r{width:100px}.big{width:100px;height:10px}", &b, 800.0);
        // The text item cannot shrink below its 80px word; the other takes the rest.
        assert_eq!(w.r(2).w, 80.0);
        assert_eq!(w.r(4).w, 20.0);
    }

    #[test]
    fn flex_wrap() {
        let mut b = Buf::default();
        b.create(1, "div").class(1, "v-row r").insert(0, 1, 0);
        for i in 2..6 {
            b.create(i, "div").class(i, "i").insert(1, i, 0);
        }
        let w = world(
            ".r{width:250px;flex-wrap:wrap;gap:10px}.i{width:100px;height:20px}",
            &b,
            800.0,
        );
        assert_eq!(
            w.r(3),
            Rect {
                x: 110.0,
                y: 0.0,
                w: 100.0,
                h: 20.0
            }
        );
        assert_eq!(
            w.r(4),
            Rect {
                x: 0.0,
                y: 30.0,
                w: 100.0,
                h: 20.0
            }
        );
        assert_eq!(w.r(1).h, 50.0);
    }

    #[test]
    fn grid_tracks_span_and_fill() {
        let mut b = Buf::default();
        b.create(1, "div").class(1, "v-grid g").insert(0, 1, 0);
        for i in 2..6 {
            b.create(i, "div")
                .class(i, if i == 3 { "w h" } else { "h" })
                .insert(1, i, 0);
        }
        let w = world(
            ".g{width:320px;grid-template-columns:repeat(3,minmax(0,1fr));gap:10px}.h{height:20px}.w{grid-column:span 2}",
            &b,
            800.0,
        );
        assert_eq!(
            w.r(2),
            Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 20.0
            }
        );
        assert_eq!(
            w.r(3),
            Rect {
                x: 110.0,
                y: 0.0,
                w: 210.0,
                h: 20.0
            }
        );
        assert_eq!(
            w.r(4),
            Rect {
                x: 0.0,
                y: 30.0,
                w: 100.0,
                h: 20.0
            }
        );
        let w2 = world(
            ".g{width:500px;grid-template-columns:repeat(auto-fill,minmax(min(150px,100%),1fr));gap:10px}.h{height:20px}",
            &b,
            800.0,
        );
        assert_eq!(w2.r(2).w, (500.0 - 20.0) / 3.0);
        let w3 = world(
            ".g{width:300px;grid-template-columns:100px 1fr}.h{height:20px}",
            &b,
            800.0,
        );
        assert_eq!(
            w3.r(3),
            Rect {
                x: 100.0,
                y: 0.0,
                w: 200.0,
                h: 20.0
            }
        );
    }

    #[test]
    fn stack_overlays_children() {
        let mut b = Buf::default();
        b.create(1, "div").class(1, "v-stack").insert(0, 1, 0);
        b.create(2, "div").class(2, "a").insert(1, 2, 0);
        b.create(3, "div").class(3, "b").insert(1, 3, 0);
        let w = world(".a{height:50px}.b{height:20px;width:20px}", &b, 300.0);
        assert_eq!(
            w.r(2),
            Rect {
                x: 0.0,
                y: 0.0,
                w: 300.0,
                h: 50.0
            }
        );
        assert_eq!(
            w.r(3),
            Rect {
                x: 0.0,
                y: 0.0,
                w: 20.0,
                h: 20.0
            }
        );
    }

    #[test]
    fn absolute_and_relative() {
        let mut b = Buf::default();
        b.create(1, "div").class(1, "p").insert(0, 1, 0);
        b.create(2, "div").class(2, "a").insert(1, 2, 0);
        b.create(3, "div").class(3, "r").insert(1, 3, 0);
        let w = world(
            ".p{position:relative;width:200px;height:100px;border:2px solid #000}.a{position:absolute;right:10px;bottom:0;width:30px;height:30px}.r{position:relative;top:5px;height:10px}",
            &b,
            800.0,
        );
        assert_eq!(
            w.r(2),
            Rect {
                x: 2.0 + 196.0 - 10.0 - 30.0,
                y: 2.0 + 96.0 - 30.0,
                w: 30.0,
                h: 30.0
            }
        );
        assert_eq!(w.t.n(3).lay.offset, (0.0, 5.0));
        assert_eq!(w.r(3).y, 2.0);
    }

    #[test]
    fn inline_atoms_share_lines() {
        let mut b = Buf::default();
        b.create(1, "p").insert(0, 1, 0);
        b.text(2, "Go ").insert(1, 2, 0);
        b.create(3, "button")
            .insert(1, 3, 0)
            .text(4, "ok")
            .insert(3, 4, 0);
        b.create(5, "strong")
            .insert(1, 5, 0)
            .text(6, " now")
            .insert(5, 6, 0);
        let w = world("", &b, 800.0);
        // "Go " is 24px; the button is 16 + 12 padding + 2 border wide.
        assert_eq!(w.r(3).x, 24.0);
        assert_eq!(w.r(3).w, 30.0);
        let f = &w.t.n(1).lay.frags;
        assert_eq!(f.len(), 2);
        assert_eq!(f[1].text, " now");
        assert_eq!(f[1].rect.x, 54.0);
    }

    #[test]
    fn scroll_extent() {
        let mut b = Buf::default();
        b.create(1, "div").class(1, "v-scroll s").insert(0, 1, 0);
        b.create(2, "div").class(2, "tall").insert(1, 2, 0);
        let mut w = world(".s{height:100px}.tall{height:400px}", &b, 300.0);
        assert_eq!(w.t.n(1).lay.content, (300.0, 400.0));
        w.t.m(1).lay.scroll.1 = 1000.0;
        w.t.mark_layout(1);
        w.run(300.0);
        assert_eq!(w.t.n(1).lay.scroll.1, 300.0);
    }

    #[test]
    fn wraps_field_text() {
        let mut f = Fonts::default();
        let s = Style::default();
        let lines = wrap_lines(&mut f, &mut Mono, &s, "aaa bbb ccc\nd", 60.0);
        assert_eq!(lines, vec![(0, 8), (8, 11), (12, 13)]);
    }
}
