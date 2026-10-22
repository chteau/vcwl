//! Computed styles: built-in element defaults, the app's rules and inheritance.

use std::collections::HashMap;
use std::rc::Rc;

use crate::css::{Cond, Pseudo, Rule, Tok};
use crate::dom::{self, Id, Kind, Node, Tree};

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Len {
    #[default]
    Auto,
    Px(f32),
    Pct(f32),
    Fr(f32),
    MinContent,
    MaxContent,
    FitContent,
    None,
}

impl Len {
    /// Resolves against `base` (the containing block size, when known).
    pub fn px(self, base: Option<f32>) -> Option<f32> {
        match self {
            Len::Px(v) => Some(v),
            Len::Pct(p) => base.map(|b| b * p / 100.0),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Display {
    #[default]
    Block,
    Inline,
    InlineBlock,
    Flex,
    InlineFlex,
    Grid,
    Contents,
    None,
}

impl Display {
    pub fn inline_level(self) -> bool {
        matches!(
            self,
            Display::Inline | Display::InlineBlock | Display::InlineFlex
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Align {
    #[default]
    Stretch,
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Justify {
    #[default]
    Start,
    Center,
    End,
    Between,
    Around,
    Evenly,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Position {
    #[default]
    Static,
    Relative,
    Absolute,
    Fixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Overflow {
    #[default]
    Visible,
    Hidden,
    Scroll,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum WhiteSpace {
    #[default]
    Normal,
    NoWrap,
    Pre,
    PreWrap,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Case {
    #[default]
    None,
    Upper,
    Lower,
    Capitalize,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Decoration {
    #[default]
    None,
    Underline,
    LineThrough,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum BorderStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
    Double,
}

/// RGBA, 8 bits per channel, red in the high byte.
pub type Color = u32;
pub const TRANSPARENT: Color = 0;
pub const BLACK: Color = 0x000000ff;
pub const WHITE: Color = 0xffffffff;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Border {
    pub width: f32,
    pub style: BorderStyle,
    pub color: Option<Color>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Background {
    #[default]
    None,
    Color(Color),
    /// Angle in degrees (CSS convention: 180 = to bottom) and stops (offset 0..1, color).
    Linear(f32, Vec<(f32, Color)>),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Tracks {
    #[default]
    None,
    Count(u32),
    Fill(f32),
    List(Vec<Len>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
    pub inset: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transform {
    Translate(Len, Len),
    Scale(f32, f32),
    Rotate(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Fit {
    #[default]
    Fill,
    Contain,
    Cover,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ListStyle {
    #[default]
    None,
    Disc,
    Decimal,
    Square,
    Circle,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Cursor {
    #[default]
    Auto,
    Default,
    Pointer,
    Text,
    NotAllowed,
    Grab,
    Move,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub display: Display,
    pub row: bool,
    pub reverse: bool,
    pub wrap: bool,
    pub align: Align,
    pub align_self: Option<Align>,
    pub justify: Justify,
    pub grow: f32,
    pub shrink: f32,
    pub basis: Len,
    pub width: Len,
    pub height: Len,
    pub min_width: Len,
    pub min_height: Len,
    pub max_width: Len,
    pub max_height: Len,
    /// Top, right, bottom, left.
    pub padding: [Len; 4],
    pub margin: [Len; 4],
    pub row_gap: f32,
    pub col_gap: f32,
    pub border: [Border; 4],
    pub radius: [f32; 4],
    pub background: Background,
    pub columns: Tracks,
    pub rows: Tracks,
    pub col_span: u32,
    /// The element and its children are laid out on top of each other in one cell.
    pub stack: bool,
    pub position: Position,
    pub inset: [Len; 4],
    pub z: i32,
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,
    pub opacity: f32,
    pub transform: Vec<Transform>,
    pub shadow: Vec<Shadow>,
    /// Shape of the corners (VCWL's `corner`): 0 round, 1 squircle, 2 cut, 3 scoop.
    pub corner: u8,
    /// Film grain over the background, 0 to 1 (VCWL's `grain`).
    pub grain: f32,
    /// A hand-drawn border (VCWL's `sketch`), drawn instead of `border`.
    pub sketch: Option<Border>,
    pub fit: Fit,
    pub container: bool,
    pub transitions: Vec<crate::anim::Transition>,
    // Inherited.
    pub color: Color,
    pub font_family: Rc<str>,
    pub font_size: f32,
    pub weight: u16,
    pub italic: bool,
    /// Line height in px.
    pub line_height: f32,
    pub letter_spacing: f32,
    pub text_align: TextAlign,
    pub decoration: Decoration,
    pub case: Case,
    pub white_space: WhiteSpace,
    pub cursor: Cursor,
    pub list_style: ListStyle,
    pub selectable: bool,
    /// CSS `font` shorthand for canvas text, derived from the font properties.
    pub font: Rc<str>,
}

const FONT: &str = "system-ui,-apple-system,'Segoe UI',sans-serif";
pub const MONO: &str = "ui-monospace,SFMono-Regular,Menlo,Consolas,monospace";
pub const FOCUS_RING: Color = 0x4c9ffeff;

impl Default for Style {
    fn default() -> Self {
        Style {
            display: Display::Inline,
            row: true,
            reverse: false,
            wrap: false,
            align: Align::Stretch,
            align_self: None,
            justify: Justify::Start,
            grow: 0.0,
            shrink: 1.0,
            basis: Len::Auto,
            width: Len::Auto,
            height: Len::Auto,
            min_width: Len::Auto,
            min_height: Len::Auto,
            max_width: Len::None,
            max_height: Len::None,
            padding: [Len::Px(0.0); 4],
            margin: [Len::Px(0.0); 4],
            row_gap: 0.0,
            col_gap: 0.0,
            border: [Border::default(); 4],
            radius: [0.0; 4],
            background: Background::None,
            columns: Tracks::None,
            rows: Tracks::None,
            col_span: 1,
            stack: false,
            position: Position::Static,
            inset: [Len::Auto; 4],
            z: 0,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            opacity: 1.0,
            transform: Vec::new(),
            shadow: Vec::new(),
            corner: 0,
            grain: 0.0,
            sketch: None,
            fit: Fit::Fill,
            container: false,
            transitions: Vec::new(),
            color: BLACK,
            font_family: FONT.into(),
            font_size: 16.0,
            weight: 400,
            italic: false,
            line_height: 24.0,
            letter_spacing: 0.0,
            text_align: TextAlign::Left,
            decoration: Decoration::None,
            case: Case::None,
            white_space: WhiteSpace::Normal,
            cursor: Cursor::Auto,
            list_style: ListStyle::None,
            selectable: true,
            font: "400 16px system-ui,-apple-system,'Segoe UI',sans-serif".into(),
        }
    }
}

impl Style {
    /// A child's starting style: inherited properties from `self`, the rest initial.
    fn inherit(&self) -> Style {
        Style {
            color: self.color,
            font_family: self.font_family.clone(),
            font_size: self.font_size,
            weight: self.weight,
            italic: self.italic,
            line_height: self.line_height,
            letter_spacing: self.letter_spacing,
            text_align: self.text_align,
            decoration: self.decoration,
            case: self.case,
            white_space: self.white_space,
            cursor: self.cursor,
            list_style: self.list_style,
            selectable: self.selectable,
            font: self.font.clone(),
            ..Style::default()
        }
    }

    pub fn clips(&self) -> bool {
        self.overflow_x != Overflow::Visible || self.overflow_y != Overflow::Visible
    }

    fn font_shorthand(&self) -> String {
        let italic = if self.italic { "italic " } else { "" };
        format!(
            "{italic}{} {}px {}",
            self.weight,
            crate::css::num(self.font_size),
            self.font_family
        )
    }
}

/// Inputs to rule matching that come from the environment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Media {
    pub width: f32,
    pub height: f32,
    pub dark: bool,
    pub reduced_motion: bool,
}

/// Settings and output of one restyle.
struct Pass<'a> {
    media: &'a Media,
    all: bool,
    prev: &'a mut Vec<(Id, Style)>,
}

pub struct Sheet {
    rules: Vec<Rule>,
    /// Rule indices by class name.
    by_class: HashMap<String, Vec<usize>>,
}

impl Sheet {
    pub fn new(src: &str) -> Sheet {
        let rules = crate::css::parse_sheet(src);
        let mut by_class: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, r) in rules.iter().enumerate() {
            by_class.entry(r.class.clone()).or_default().push(i);
        }
        Sheet { rules, by_class }
    }

    /// Recomputes styles: of every node if `all`, else of nodes marked `style_dirty` and of
    /// descendants whose inherited values changed. Nodes whose style changed get their
    /// layout invalidated.
    pub fn restyle(&self, t: &mut Tree, media: &Media, all: bool) {
        self.restyle_tracked(t, media, all, &mut Vec::new());
    }

    /// Like `restyle`, collecting the previous style of nodes with transitions whose style
    /// changed.
    pub fn restyle_tracked(
        &self,
        t: &mut Tree,
        media: &Media,
        all: bool,
        prev: &mut Vec<(Id, Style)>,
    ) {
        let mut pass = Pass { media, all, prev };
        self.update(t, 0, all, None, &mut pass);
    }

    /// Whether `class` has `when appear` rules.
    pub fn appears(&self, class: &str) -> bool {
        self.by_class.get(class).is_some_and(|rs| {
            rs.iter()
                .any(|&i| self.rules[i].pseudo.contains(&Pseudo::Appear))
        })
    }

    pub fn uses_appear(&self) -> bool {
        self.rules
            .iter()
            .any(|r| r.pseudo.contains(&Pseudo::Appear))
    }

    fn update(&self, t: &mut Tree, id: Id, force: bool, container: Option<f32>, p: &mut Pass) {
        let (all, media) = (p.all, p.media);
        let mut changed = false;
        if all || force || t.n(id).style_dirty {
            let base = match t.n(id).parent {
                Some(p) => t.n(p).style.inherit(),
                None => Style::default(),
            };
            let s = self.compute(t, id, base, media, container);
            let n = t.m(id);
            n.style_dirty = false;
            let styled = n.flags & dom::STYLED != 0;
            n.flags |= dom::STYLED;
            if n.style != s {
                if styled && !(s.transitions.is_empty() && n.style.transitions.is_empty()) {
                    p.prev.push((id, n.style.clone()));
                }
                n.style = s;
                changed = true;
                t.mark_layout(id);
            }
        }
        let container = if t.n(id).style.container {
            t.n(id).lay.content_width()
        } else {
            container
        };
        for i in 0..t.n(id).children.len() {
            let c = t.n(id).children[i];
            self.update(t, c, changed, container, p);
        }
    }

    fn compute(
        &self,
        t: &Tree,
        id: Id,
        mut s: Style,
        media: &Media,
        container: Option<f32>,
    ) -> Style {
        let n = t.n(id);
        match n.kind {
            Kind::Element => {
                // `em` font sizes are relative to the parent's size, not the default for the tag.
                let font = s.font_size;
                defaults(n, &mut s);
                let (default_font, default_margin) = (s.font_size, s.margin);
                let checked = n.checked
                    || (n.has_class("v-check")
                        && n.children.first().is_some_and(|&c| t.n(c).checked));
                let mut matched: Vec<(usize, usize)> = Vec::new();
                for c in &n.classes {
                    for &i in self.by_class.get(c).map(|v| v.as_slice()).unwrap_or(&[]) {
                        let r = &self.rules[i];
                        if r.conds.iter().all(|c| cond(*c, media, container))
                            && r.pseudo.iter().all(|p| pseudo(*p, n, checked))
                        {
                            matched.push((r.pseudo.len(), i));
                        }
                    }
                }
                matched.sort();
                matched.dedup();
                for pass in 0..2 {
                    for &(_, i) in &matched {
                        for (prop, v) in &self.rules[i].decls {
                            if (prop == "font-size") == (pass == 0) {
                                let parent_font = if pass == 0 { font } else { s.font_size };
                                apply(&mut s, prop, v, parent_font, media);
                            }
                        }
                    }
                }
                // `style bind` values, after the classes.
                for (k, v) in &n.attrs {
                    if let Some((prop, unit)) =
                        k.strip_prefix("style:").and_then(|r| r.split_once(':'))
                    {
                        let toks = crate::css::tokens(&format!("{v}{unit}"));
                        let own = s.font_size;
                        apply(&mut s, prop, &toks, own, media);
                    }
                }
                // Default margins are in `em` of the element's own (possibly restyled) size.
                if s.font_size != default_font && s.margin == default_margin {
                    let em = match n.tag.as_str() {
                        "p" | "ul" | "ol" | "h3" => 1.0,
                        "h1" => 0.67,
                        "h2" => 0.83,
                        "h4" => 1.33,
                        "h5" => 1.67,
                        "h6" => 2.33,
                        _ => 0.0,
                    };
                    if em > 0.0 {
                        let m = px(em * s.font_size);
                        s.margin = [m, px(0.0), m, px(0.0)];
                    }
                }
                s.font = s.font_shorthand().into();
            }
            Kind::Text => s.display = Display::Inline,
            Kind::Comment => s.display = Display::None,
        }
        s
    }

    pub fn uses_container_queries(&self) -> bool {
        self.rules.iter().any(|r| {
            r.conds
                .iter()
                .any(|c| matches!(c, Cond::ContainerMax(_) | Cond::ContainerMin(_)))
        })
    }
}

fn cond(c: Cond, m: &Media, container: Option<f32>) -> bool {
    match c {
        Cond::MaxWidth(w) => m.width <= w,
        Cond::MinWidth(w) => m.width >= w,
        Cond::ContainerMax(w) => container.is_some_and(|c| c <= w),
        Cond::ContainerMin(w) => container.is_some_and(|c| c >= w),
        Cond::Dark => m.dark,
        Cond::Light => !m.dark,
        Cond::ReducedMotion => m.reduced_motion,
        Cond::Print => false,
    }
}

fn pseudo(p: Pseudo, n: &Node, checked: bool) -> bool {
    match p {
        Pseudo::Hover => n.flags & dom::HOVER != 0,
        Pseudo::Active => n.flags & dom::ACTIVE != 0,
        Pseudo::FocusVisible => n.flags & dom::FOCUS_VISIBLE != 0,
        Pseudo::FocusWithin => n.flags & (dom::FOCUS_WITHIN | dom::FOCUS) != 0,
        Pseudo::Disabled => n.attr("disabled").is_some(),
        Pseudo::Checked => checked,
        Pseudo::Appear => n.flags & dom::SEEN == 0,
    }
}

fn px(v: f32) -> Len {
    Len::Px(v)
}

/// Built-in styles for each element, following browser defaults.
fn defaults(n: &Node, s: &mut Style) {
    use Display::*;
    let fs = s.font_size;
    s.display = match n.tag.as_str() {
        "body" => {
            s.font_size = 16.0;
            s.line_height = 24.0;
            Block
        }
        "div" | "section" | "header" | "footer" | "nav" | "main" | "article" | "aside" | "form"
        | "li" => Block,
        "p" => {
            s.margin = [px(fs), px(0.0), px(fs), px(0.0)];
            Block
        }
        "ul" | "ol" => {
            s.margin = [px(fs), px(0.0), px(fs), px(0.0)];
            s.padding[3] = px(40.0);
            s.list_style = if n.tag == "ul" {
                ListStyle::Disc
            } else {
                ListStyle::Decimal
            };
            Block
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let (scale, m) = match n.tag.as_bytes()[1] {
                b'1' => (2.0, 0.67),
                b'2' => (1.5, 0.83),
                b'3' => (1.17, 1.0),
                b'4' => (1.0, 1.33),
                b'5' => (0.83, 1.67),
                _ => (0.67, 2.33),
            };
            s.font_size = fs * scale;
            s.line_height = s.font_size * 1.5;
            s.weight = 700;
            let m = px(s.font_size * m);
            s.margin = [m, px(0.0), m, px(0.0)];
            Block
        }
        "strong" => {
            s.weight = 700;
            Inline
        }
        "code" => {
            s.font_family = MONO.into();
            s.font_size = fs * 0.8125;
            s.line_height *= 0.8125;
            Inline
        }
        "a" => {
            s.color = 0x0b57d0ff;
            s.decoration = Decoration::Underline;
            s.cursor = Cursor::Pointer;
            Inline
        }
        "button" => {
            control(s);
            s.padding = [px(1.0), px(6.0), px(1.0), px(6.0)];
            s.background = Background::Color(if n.flags & dom::HOVER != 0 {
                0xe5e5e5ff
            } else {
                0xefefefff
            });
            s.radius = [4.0; 4];
            s.text_align = TextAlign::Center;
            s.cursor = Cursor::Pointer;
            if n.attr("disabled").is_some() {
                s.color = 0x6d6d6dff;
                s.cursor = Cursor::Default;
            }
            InlineBlock
        }
        "input" if n.attr("type") == Some("checkbox") => {
            s.width = px(13.0);
            s.height = px(13.0);
            s.margin = [px(3.0), px(3.0), px(3.0), px(4.0)];
            InlineBlock
        }
        "input" | "textarea" | "select" => {
            control(s);
            s.padding = [px(1.0), px(2.0), px(1.0), px(2.0)];
            s.background = Background::Color(WHITE);
            s.radius = [2.0; 4];
            s.cursor = if n.tag == "select" {
                Cursor::Default
            } else {
                Cursor::Text
            };
            if n.tag == "textarea" {
                s.white_space = WhiteSpace::PreWrap;
                s.overflow_y = Overflow::Scroll;
            }
            if n.tag == "select" {
                s.radius = [4.0; 4];
            }
            InlineBlock
        }
        "img" => InlineBlock,
        // A 3D scene is a box of the page; its content is drawn by the host (`Engine::scene`).
        "scene" => {
            s.height = px(300.0);
            Block
        }
        "mesh" | "light" | "group" => None,
        "hr" => {
            s.border[0] = Border {
                width: 1.0,
                style: BorderStyle::Solid,
                color: Some(0x8f8f8fff),
            };
            s.margin = [px(8.0), px(0.0), px(8.0), px(0.0)];
            Block
        }
        "label" | "span" => Inline,
        "option" => None,
        _ => Inline,
    };
    for c in &n.classes {
        match c.as_str() {
            "v-col" => {
                s.display = Flex;
                s.row = false;
            }
            "v-row" => s.display = Flex,
            "v-stack" => {
                s.display = Grid;
                s.stack = true;
            }
            "v-grid" => s.display = Grid,
            "v-scroll" => {
                s.overflow_x = Overflow::Scroll;
                s.overflow_y = Overflow::Scroll;
            }
            "v-list" => {
                s.margin = [px(0.0); 4];
                s.padding[3] = px(1.25 * fs);
            }
            "v-check" => {
                s.display = InlineFlex;
                s.col_gap = 0.5 * fs;
                s.align = Align::Center;
                s.cursor = Cursor::Pointer;
            }
            _ => {}
        }
    }
}

fn control(s: &mut Style) {
    let b = Border {
        width: 1.0,
        style: BorderStyle::Solid,
        color: Some(0x767676ff),
    };
    s.border = [b; 4];
    s.color = BLACK;
    s.decoration = Decoration::None;
    s.case = Case::None;
}

fn len(t: &Tok, font: f32, m: &Media) -> Option<Len> {
    Some(match t {
        Tok::Num(n, u) => match u.as_str() {
            "" | "px" => Len::Px(*n),
            "rem" => Len::Px(n * 16.0),
            "em" => Len::Px(n * font),
            "ch" => Len::Px(n * font * 0.55),
            "%" => Len::Pct(*n),
            "vw" => Len::Px(n * m.width / 100.0),
            "vh" | "dvh" | "svh" => Len::Px(n * m.height / 100.0),
            "vmin" => Len::Px(n * m.width.min(m.height) / 100.0),
            "vmax" => Len::Px(n * m.width.max(m.height) / 100.0),
            "fr" => Len::Fr(*n),
            _ => return None,
        },
        Tok::Ident(k) => match k.as_str() {
            "auto" => Len::Auto,
            "none" => Len::None,
            "min-content" => Len::MinContent,
            "max-content" => Len::MaxContent,
            "fit-content" => Len::FitContent,
            _ => return None,
        },
        _ => return None,
    })
}

/// Expands 1–4 values to top, right, bottom, left.
fn sides<T: Copy>(v: &[T]) -> Option<[T; 4]> {
    Some(match *v {
        [a] => [a, a, a, a],
        [a, b] => [a, b, a, b],
        [a, b, c] => [a, b, c, b],
        [a, b, c, d] => [a, b, c, d],
        _ => return None,
    })
}

fn px_of(l: Len) -> f32 {
    match l {
        Len::Px(v) => v,
        _ => 0.0,
    }
}

pub fn color(t: &Tok, current: Color) -> Option<Color> {
    match t {
        Tok::Hash(h) => {
            let d: Vec<u32> = h.chars().map(|c| c.to_digit(16)).collect::<Option<_>>()?;
            Some(match d.len() {
                3 => ((d[0] * 17) << 24) | ((d[1] * 17) << 16) | ((d[2] * 17) << 8) | 0xff,
                4 => ((d[0] * 17) << 24) | ((d[1] * 17) << 16) | ((d[2] * 17) << 8) | (d[3] * 17),
                6 => (u32::from_str_radix(h, 16).ok()? << 8) | 0xff,
                8 => u32::from_str_radix(h, 16).ok()?,
                _ => return None,
            })
        }
        Tok::Ident(k) => match k.as_str() {
            "transparent" => Some(TRANSPARENT),
            "currentcolor" | "inherit" => Some(current),
            "white" => Some(WHITE),
            "black" => Some(BLACK),
            "red" => Some(0xff0000ff),
            _ => None,
        },
        Tok::Func(f, args) => {
            let nums: Vec<(f32, &str)> = args
                .iter()
                .filter_map(|a| match a {
                    Tok::Num(n, u) => Some((*n, u.as_str())),
                    _ => None,
                })
                .collect();
            if nums.len() < 3 {
                return None;
            }
            let alpha = nums
                .get(3)
                .map(|(a, u)| if *u == "%" { a / 100.0 } else { *a })
                .unwrap_or(1.0);
            let (r, g, b) = match f.as_str() {
                "rgb" | "rgba" => {
                    let ch = |(v, u): (f32, &str)| if u == "%" { v * 2.55 } else { v };
                    (ch(nums[0]), ch(nums[1]), ch(nums[2]))
                }
                "hsl" | "hsla" => hsl(nums[0].0, nums[1].0 / 100.0, nums[2].0 / 100.0),
                _ => return None,
            };
            let c = |v: f32| v.round().clamp(0.0, 255.0) as u32;
            Some((c(r) << 24) | (c(g) << 16) | (c(b) << 8) | c(alpha * 255.0))
        }
        _ => None,
    }
}

fn hsl(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let a = s * l.min(1.0 - l);
    let f = |n: f32| {
        let k = (n + h / 30.0).rem_euclid(12.0);
        (l - a * crate::layout::clamp((k - 3.0).min(9.0 - k), -1.0, 1.0)) * 255.0
    };
    (f(0.0), f(8.0), f(4.0))
}

fn align(k: &str) -> Option<Align> {
    Some(match k {
        "flex-start" | "start" | "baseline" => Align::Start,
        "center" => Align::Center,
        "flex-end" | "end" => Align::End,
        "stretch" => Align::Stretch,
        _ => return None,
    })
}

fn split_commas(v: &[Tok]) -> Vec<&[Tok]> {
    v.split(|t| *t == Tok::Comma).collect()
}

fn gradient(args: &[Tok], current: Color) -> Option<Background> {
    let groups = split_commas(args);
    let mut angle = 180.0;
    let mut stops_at = 0;
    match groups.first()? {
        [Tok::Num(n, u)] if u == "deg" || u == "turn" => {
            angle = if u == "turn" { n * 360.0 } else { *n };
            stops_at = 1;
        }
        [Tok::Ident(to), rest @ ..] if to == "to" => {
            let has = |k: &str| rest.iter().any(|t| *t == Tok::Ident(k.into()));
            angle = match (has("top"), has("bottom"), has("left"), has("right")) {
                (true, _, true, _) => 315.0,
                (true, _, _, true) => 45.0,
                (_, true, true, _) => 225.0,
                (_, true, _, true) => 135.0,
                (true, ..) => 0.0,
                (_, _, true, _) => 270.0,
                (_, _, _, true) => 90.0,
                _ => 180.0,
            };
            stops_at = 1;
        }
        _ => {}
    }
    let raw: Vec<(Color, Option<f32>)> = groups[stops_at..]
        .iter()
        .filter_map(|g| {
            let c = color(g.first()?, current)?;
            let at = match g.get(1) {
                Some(Tok::Num(n, u)) if u == "%" => Some(n / 100.0),
                _ => None,
            };
            Some((c, at))
        })
        .collect();
    if raw.is_empty() {
        return None;
    }
    let last = (raw.len() - 1).max(1) as f32;
    let stops = raw
        .iter()
        .enumerate()
        .map(|(i, (c, at))| (at.unwrap_or(i as f32 / last), *c))
        .collect();
    Some(Background::Linear(angle, stops))
}

fn tracks(v: &[Tok], font: f32, m: &Media) -> Tracks {
    match v {
        [Tok::Func(f, args)] if f == "repeat" => match args.as_slice() {
            [Tok::Num(n, _), Tok::Comma, ..] => Tracks::Count(*n as u32),
            [Tok::Ident(k), Tok::Comma, Tok::Func(mm, inner)]
                if k == "auto-fill" && mm == "minmax" =>
            {
                match inner.first() {
                    Some(Tok::Func(min, a)) if min == "min" => Tracks::Fill(
                        a.first()
                            .and_then(|t| len(t, font, m))
                            .map(px_of)
                            .unwrap_or(0.0),
                    ),
                    Some(t) => Tracks::Fill(len(t, font, m).map(px_of).unwrap_or(0.0)),
                    None => Tracks::None,
                }
            }
            _ => Tracks::None,
        },
        _ => Tracks::List(v.iter().filter_map(|t| len(t, font, m)).collect()),
    }
}

fn border(v: &[Tok], current: Color, font: f32, m: &Media) -> Border {
    let mut b = Border {
        width: 0.0,
        style: BorderStyle::Solid,
        color: None,
    };
    if matches!(v, [Tok::Ident(k)] if k == "none") {
        return b;
    }
    b.width = 1.0;
    for t in v {
        match t {
            Tok::Ident(k) if k == "dashed" => b.style = BorderStyle::Dashed,
            Tok::Ident(k) if k == "dotted" => b.style = BorderStyle::Dotted,
            Tok::Ident(k) if k == "double" => b.style = BorderStyle::Double,
            Tok::Ident(k) if k == "solid" => {}
            Tok::Num(..) => b.width = len(t, font, m).map(px_of).unwrap_or(1.0),
            _ => b.color = color(t, current).or(b.color),
        }
    }
    b
}

/// Applies one declaration. Values were validated by the compiler; anything else is ignored.
fn apply(s: &mut Style, prop: &str, v: &[Tok], parent_font: f32, m: &Media) {
    let font = s.font_size;
    let l = |i: usize| v.get(i).and_then(|t| len(t, font, m));
    let lens = || v.iter().filter_map(|t| len(t, font, m)).collect::<Vec<_>>();
    let kw = || match v.first() {
        Some(Tok::Ident(k)) => k.as_str(),
        _ => "",
    };
    let num = || match v.first() {
        Some(Tok::Num(n, _)) => Some(*n),
        _ => None,
    };
    match prop {
        "color" => {
            if let Some(c) = v.first().and_then(|t| color(t, s.color)) {
                s.color = c;
            }
        }
        "background" => {
            s.background = match v.first() {
                Some(Tok::Func(f, args)) if f == "linear-gradient" => {
                    gradient(args, s.color).unwrap_or_default()
                }
                Some(Tok::Func(f, args)) if f == "radial-gradient" => match gradient(args, s.color)
                {
                    Some(Background::Linear(_, stops)) => Background::Color(stops[0].1),
                    _ => Background::None,
                },
                Some(t) => color(t, s.color).map(Background::Color).unwrap_or_default(),
                None => Background::None,
            }
        }
        "width" => s.width = l(0).unwrap_or_default(),
        "height" => s.height = l(0).unwrap_or_default(),
        "min-width" => s.min_width = l(0).unwrap_or_default(),
        "min-height" => s.min_height = l(0).unwrap_or_default(),
        "max-width" => s.max_width = l(0).unwrap_or(Len::None),
        "max-height" => s.max_height = l(0).unwrap_or(Len::None),
        "padding" => s.padding = sides(&lens()).unwrap_or(s.padding),
        "padding-inline" => {
            let x = lens();
            if let Some(&a) = x.first() {
                s.padding[3] = a;
                s.padding[1] = *x.get(1).unwrap_or(&a);
            }
        }
        "padding-block" => {
            let x = lens();
            if let Some(&a) = x.first() {
                s.padding[0] = a;
                s.padding[2] = *x.get(1).unwrap_or(&a);
            }
        }
        "padding-top" | "padding-right" | "padding-bottom" | "padding-left" => {
            s.padding[side(prop)] = l(0).unwrap_or_default()
        }
        "margin" => s.margin = sides(&lens()).unwrap_or(s.margin),
        "margin-inline" => {
            let x = lens();
            if let Some(&a) = x.first() {
                s.margin[3] = a;
                s.margin[1] = *x.get(1).unwrap_or(&a);
            }
        }
        "margin-block" => {
            let x = lens();
            if let Some(&a) = x.first() {
                s.margin[0] = a;
                s.margin[2] = *x.get(1).unwrap_or(&a);
            }
        }
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left" => {
            s.margin[side(prop)] = l(0).unwrap_or_default()
        }
        "gap" => {
            let x = lens();
            if let Some(&a) = x.first() {
                s.row_gap = px_of(a);
                s.col_gap = px_of(*x.get(1).unwrap_or(&a));
            }
        }
        "row-gap" => s.row_gap = l(0).map(px_of).unwrap_or(0.0),
        "column-gap" => s.col_gap = l(0).map(px_of).unwrap_or(0.0),
        "border" => s.border = [border(v, s.color, font, m); 4],
        "border-top" | "border-right" | "border-bottom" | "border-left" => {
            s.border[side(prop)] = border(v, s.color, font, m)
        }
        "-vcwl-corner" => {
            s.corner = match v.first() {
                Some(Tok::Ident(k)) if k == "squircle" => 1,
                Some(Tok::Ident(k)) if k == "cut" => 2,
                Some(Tok::Ident(k)) if k == "scoop" => 3,
                _ => 0,
            }
        }
        "-vcwl-grain" => s.grain = num().unwrap_or(0.0).clamp(0.0, 1.0),
        "-vcwl-sketch" => s.sketch = Some(border(v, s.color, font, m)).filter(|b| b.width > 0.0),
        "border-radius" => {
            if let Some(r) = sides(&lens()) {
                s.radius = r.map(px_of);
            }
        }
        "font-family" => {
            let mut out = String::new();
            for t in v {
                match t {
                    Tok::Ident(k) => {
                        if !out.is_empty() && !out.ends_with(',') {
                            out.push(' ');
                        }
                        out.push_str(k);
                    }
                    Tok::Str(q) => {
                        out.push('"');
                        out.push_str(q);
                        out.push('"');
                    }
                    Tok::Comma => out.push(','),
                    _ => {}
                }
            }
            s.font_family = out.into();
        }
        "font-size" => {
            if let Some(Tok::Num(n, u)) = v.first() {
                let new = match u.as_str() {
                    "em" => n * parent_font,
                    "%" => n * parent_font / 100.0,
                    _ => len(&v[0], parent_font, m).map(px_of).unwrap_or(parent_font),
                };
                let ratio = s.line_height / s.font_size.max(1.0);
                s.font_size = new;
                s.line_height = new * ratio;
            }
        }
        "font-weight" => {
            s.weight = match v.first() {
                Some(Tok::Num(n, _)) => *n as u16,
                Some(Tok::Ident(k)) if k == "bold" => 700,
                _ => 400,
            }
        }
        "line-height" => {
            s.line_height = match v.first() {
                Some(Tok::Num(n, u)) if u.is_empty() => n * font,
                Some(t) => len(t, font, m).map(px_of).unwrap_or(font * 1.5),
                None => font * 1.5,
            }
        }
        "letter-spacing" => s.letter_spacing = l(0).map(px_of).unwrap_or(0.0),
        "font-style" => s.italic = kw() == "italic",
        "text-align" => {
            s.text_align = match kw() {
                "center" => TextAlign::Center,
                "right" | "end" => TextAlign::Right,
                _ => TextAlign::Left,
            }
        }
        "text-decoration" => {
            s.decoration = match kw() {
                "underline" => Decoration::Underline,
                "line-through" => Decoration::LineThrough,
                _ => Decoration::None,
            }
        }
        "text-transform" => {
            s.case = match kw() {
                "uppercase" => Case::Upper,
                "lowercase" => Case::Lower,
                "capitalize" => Case::Capitalize,
                _ => Case::None,
            }
        }
        "white-space" => {
            s.white_space = match kw() {
                "nowrap" => WhiteSpace::NoWrap,
                "pre" => WhiteSpace::Pre,
                "pre-wrap" => WhiteSpace::PreWrap,
                _ => WhiteSpace::Normal,
            }
        }
        "flex-direction" => {
            let k = kw();
            s.row = k.starts_with("row");
            s.reverse = k.ends_with("reverse");
        }
        "align-items" => s.align = align(kw()).unwrap_or_default(),
        "align-self" => s.align_self = align(kw()),
        "justify-content" => {
            s.justify = match kw() {
                "center" => Justify::Center,
                "flex-end" | "end" => Justify::End,
                "space-between" => Justify::Between,
                "space-around" => Justify::Around,
                "space-evenly" => Justify::Evenly,
                _ => Justify::Start,
            }
        }
        "flex-wrap" => s.wrap = kw() == "wrap",
        "flex-grow" => s.grow = num().unwrap_or(0.0),
        "flex-shrink" => s.shrink = num().unwrap_or(1.0),
        "flex-basis" => s.basis = l(0).unwrap_or_default(),
        "display" => {
            s.display = match kw() {
                "flex" => Display::Flex,
                "grid" => Display::Grid,
                "block" => Display::Block,
                "inline" => Display::Inline,
                "inline-block" => Display::InlineBlock,
                "inline-flex" => Display::InlineFlex,
                "contents" => Display::Contents,
                _ => Display::None,
            }
        }
        "grid-template-columns" => s.columns = tracks(v, font, m),
        "grid-template-rows" => s.rows = tracks(v, font, m),
        "grid-column" => {
            if let [Tok::Ident(k), Tok::Num(n, _)] = v
                && k == "span"
            {
                s.col_span = (*n as u32).max(1);
            }
        }
        "position" => {
            s.position = match kw() {
                "relative" | "sticky" => Position::Relative,
                "absolute" => Position::Absolute,
                "fixed" => Position::Fixed,
                _ => Position::Static,
            }
        }
        "top" | "right" | "bottom" | "left" => s.inset[side(prop)] = l(0).unwrap_or_default(),
        "inset" => s.inset = sides(&lens()).unwrap_or(s.inset),
        "z-index" => s.z = num().unwrap_or(0.0) as i32,
        "overflow" => {
            let o = overflow(kw());
            s.overflow_x = o;
            s.overflow_y = o;
        }
        "overflow-x" => s.overflow_x = overflow(kw()),
        "overflow-y" => s.overflow_y = overflow(kw()),
        "opacity" => s.opacity = num().unwrap_or(1.0),
        "transform" => {
            s.transform = v
                .iter()
                .filter_map(|t| {
                    let Tok::Func(f, args) = t else { return None };
                    let a: Vec<&Tok> = args.iter().filter(|a| **a != Tok::Comma).collect();
                    let n = |i: usize| match a.get(i) {
                        Some(Tok::Num(n, _)) => Some(*n),
                        _ => None,
                    };
                    let l = |i: usize| {
                        a.get(i)
                            .and_then(|t| len(t, font, m))
                            .unwrap_or(Len::Px(0.0))
                    };
                    Some(match f.as_str() {
                        "scale" => Transform::Scale(n(0)?, n(1).or(n(0))?),
                        "scaleX" => Transform::Scale(n(0)?, 1.0),
                        "scaleY" => Transform::Scale(1.0, n(0)?),
                        "rotate" => match a.first() {
                            Some(Tok::Num(d, u)) => {
                                Transform::Rotate(if u == "turn" { d * 360.0 } else { *d })
                            }
                            _ => return None,
                        },
                        "translate" => Transform::Translate(l(0), l(1)),
                        "translateX" => Transform::Translate(l(0), Len::Px(0.0)),
                        "translateY" => Transform::Translate(Len::Px(0.0), l(0)),
                        _ => return None,
                    })
                })
                .collect()
        }
        "cursor" => {
            s.cursor = match kw() {
                "pointer" => Cursor::Pointer,
                "default" => Cursor::Default,
                "text" => Cursor::Text,
                "not-allowed" => Cursor::NotAllowed,
                "grab" => Cursor::Grab,
                "move" => Cursor::Move,
                _ => Cursor::Auto,
            }
        }
        "box-shadow" => {
            s.shadow = split_commas(v)
                .into_iter()
                .filter_map(|g| {
                    let ls: Vec<f32> = g
                        .iter()
                        .filter(|t| matches!(t, Tok::Num(..)))
                        .filter_map(|t| len(t, font, m))
                        .map(px_of)
                        .collect();
                    if ls.len() < 2 {
                        return None;
                    }
                    let c = g
                        .iter()
                        .find_map(|t| color(t, s.color))
                        .unwrap_or(0x00000080);
                    let inset = g.contains(&Tok::Ident("inset".into()));
                    Some(Shadow {
                        x: ls[0],
                        y: ls[1],
                        blur: *ls.get(2).unwrap_or(&0.0),
                        spread: *ls.get(3).unwrap_or(&0.0),
                        color: c,
                        inset,
                    })
                })
                .collect()
        }
        "object-fit" => {
            s.fit = match kw() {
                "contain" => Fit::Contain,
                "cover" => Fit::Cover,
                "none" => Fit::None,
                _ => Fit::Fill,
            }
        }
        "container-type" => s.container = kw() != "normal",
        "list-style" => {
            s.list_style = match kw() {
                "disc" => ListStyle::Disc,
                "decimal" => ListStyle::Decimal,
                "square" => ListStyle::Square,
                "circle" => ListStyle::Circle,
                _ => ListStyle::None,
            }
        }
        "user-select" => s.selectable = kw() != "none",
        "transition" => s.transitions = crate::anim::parse(v),
        _ => {}
    }
}

fn side(prop: &str) -> usize {
    if prop.ends_with("top") {
        0
    } else if prop.ends_with("right") {
        1
    } else if prop.ends_with("bottom") {
        2
    } else {
        3
    }
}

fn overflow(k: &str) -> Overflow {
    match k {
        "hidden" | "clip" => Overflow::Hidden,
        "auto" | "scroll" => Overflow::Scroll,
        _ => Overflow::Visible,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::tests::Buf;

    fn media() -> Media {
        Media {
            width: 800.0,
            height: 600.0,
            dark: false,
            reduced_motion: false,
        }
    }

    fn styled(css: &str, b: &Buf, m: &Media) -> Tree {
        let mut t = Tree::default();
        t.apply(&b.0, &mut Vec::new()).unwrap();
        Sheet::new(css).restyle(&mut t, m, true);
        t
    }

    #[test]
    fn colors() {
        assert_eq!(color(&Tok::Hash("fff".into()), 0), Some(0xffffffff));
        assert_eq!(color(&Tok::Hash("0003".into()), 0), Some(0x00000033));
        assert_eq!(color(&Tok::Hash("b6ff6c".into()), 0), Some(0xb6ff6cff));
        let rgb = crate::css::tokens("rgb(255 0 0 / 50%)");
        assert_eq!(color(&rgb[0], 0), Some(0xff000080));
        let h = crate::css::tokens("hsl(120,100%,50%)");
        assert_eq!(color(&h[0], 0), Some(0x00ff00ff));
    }

    #[test]
    fn cascade_and_inheritance() {
        let mut b = Buf::default();
        b.create(1, "div").class(1, "v-col a b").insert(0, 1, 0);
        b.create(2, "p")
            .insert(1, 2, 0)
            .text(3, "hi")
            .insert(2, 3, 0);
        let css = ".a{color:#ff0000;font-size:2rem;padding:4px 8px}.b{color:#00ff00}.a:hover{color:#0000ff}";
        let t = styled(css, &b, &media());
        let s = &t.n(1).style;
        assert_eq!(s.display, Display::Flex);
        assert!(!s.row);
        assert_eq!(s.color, 0x00ff00ff);
        assert_eq!(s.font_size, 32.0);
        assert_eq!(s.line_height, 48.0);
        assert_eq!(
            s.padding,
            [Len::Px(4.0), Len::Px(8.0), Len::Px(4.0), Len::Px(8.0)]
        );
        // `p` inherits color and font size; its default margin is 1em of its own font size.
        assert_eq!(t.n(2).style.color, 0x00ff00ff);
        assert_eq!(t.n(2).style.margin[0], Len::Px(32.0));
        assert_eq!(t.n(3).style.display, Display::Inline);
        // ... also when a class changes that size; `em` sizes are relative to the parent.
        let mut b = Buf::default();
        b.create(1, "p").class(1, "small").insert(0, 1, 0);
        b.create(2, "h1").class(2, "half").insert(0, 2, 0);
        let t = styled(".small{font-size:12px}.half{font-size:.5em}", &b, &media());
        assert_eq!(t.n(1).style.margin[0], Len::Px(12.0));
        assert_eq!(t.n(2).style.font_size, 8.0);
    }

    #[test]
    fn pseudo_classes_win_by_specificity() {
        let mut b = Buf::default();
        b.create(1, "button").class(1, "h x").insert(0, 1, 0);
        let mut t = Tree::default();
        t.apply(&b.0, &mut Vec::new()).unwrap();
        let sheet = Sheet::new(".h:hover{color:#0000ff}.x{color:#ff0000}");
        sheet.restyle(&mut t, &media(), true);
        assert_eq!(t.n(1).style.color, 0xff0000ff);
        t.m(1).flags |= dom::HOVER;
        t.m(1).style_dirty = true;
        sheet.restyle(&mut t, &media(), false);
        assert_eq!(t.n(1).style.color, 0x0000ffff);
    }

    #[test]
    fn media_conditions() {
        let mut b = Buf::default();
        b.create(1, "div").class(1, "a").insert(0, 1, 0);
        let css = ".a{width:100px}@media (max-width:599.98px){.a{width:50%}}@media (prefers-color-scheme:dark){.a{color:#fff}}";
        let t = styled(css, &b, &media());
        assert_eq!(t.n(1).style.width, Len::Px(100.0));
        let narrow = Media {
            width: 400.0,
            dark: true,
            ..media()
        };
        let t = styled(css, &b, &narrow);
        assert_eq!(t.n(1).style.width, Len::Pct(50.0));
        assert_eq!(t.n(1).style.color, WHITE);
    }

    #[test]
    fn values() {
        let mut b = Buf::default();
        b.create(1, "div").class(1, "a").insert(0, 1, 0);
        let css = ".a{border:2px dashed #333;border-radius:8px;background:linear-gradient(to right,#000,#fff);\
                   grid-template-columns:repeat(auto-fill,minmax(min(220px,100%),1fr));box-shadow:0 2px 8px #0003;\
                   transform:translateY(-2px) scale(1.1);grid-column:span 2;width:50vw}";
        let t = styled(css, &b, &media());
        let s = &t.n(1).style;
        assert_eq!(
            s.border[0],
            Border {
                width: 2.0,
                style: BorderStyle::Dashed,
                color: Some(0x333333ff)
            }
        );
        assert_eq!(s.radius, [8.0; 4]);
        assert_eq!(
            s.background,
            Background::Linear(90.0, vec![(0.0, BLACK), (1.0, WHITE)])
        );
        assert_eq!(s.columns, Tracks::Fill(220.0));
        assert_eq!(
            s.shadow[0],
            Shadow {
                x: 0.0,
                y: 2.0,
                blur: 8.0,
                spread: 0.0,
                color: 0x00000033,
                inset: false
            }
        );
        assert_eq!(
            s.transform,
            vec![
                Transform::Translate(Len::Px(0.0), Len::Px(-2.0)),
                Transform::Scale(1.1, 1.1)
            ]
        );
        assert_eq!(s.col_span, 2);
        assert_eq!(s.width, Len::Px(400.0));
    }
}
