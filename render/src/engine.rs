//! The renderer's state machine: applies commands, runs style, layout and paint when
//! needed, and turns pointer and keyboard input into application events.

use std::collections::{HashMap, VecDeque};

use crate::anim::{self, Anim};

use crate::css::num;
use crate::dom::{self, Effect, Id, Kind, Tree};
use crate::layout::{self, Cx, Rect, clamp};
use crate::paint::{self, Edit, Hit, Painter, Popup};
use crate::style::{Cursor, Media, Overflow, Position, Sheet};
use crate::text::{Fonts, Measure};

pub mod semantics;

#[derive(Debug, PartialEq)]
pub enum Payload {
    None,
    Str(String),
    Bool(bool),
}

/// Work for the host, in order.
#[derive(Debug, PartialEq)]
pub enum Out {
    Emit {
        node: Id,
        slot: i32,
        payload: Payload,
    },
    Title(String),
    Navigate(String),
    Open {
        url: String,
        new_tab: bool,
    },
    Warn(String),
    Fetch {
        slot: i32,
        method: i32,
        url: String,
        body: Option<String>,
    },
    LoadImage {
        id: u32,
        url: String,
    },
    Cursor(&'static str),
    Copy(String),
}

pub mod out {
    pub const EMIT: i32 = 1;
    pub const TITLE: i32 = 2;
    pub const NAVIGATE: i32 = 3;
    pub const OPEN: i32 = 4;
    pub const WARN: i32 = 5;
    pub const FETCH: i32 = 6;
    pub const LOAD_IMAGE: i32 = 7;
    pub const CURSOR: i32 = 8;
    pub const COPY: i32 = 9;
}

const BLINK_MS: f64 = 530.0;
const DOUBLE_CLICK_MS: f64 = 500.0;
const LINE_SCROLL: f32 = 40.0;

struct Down {
    target: Id,
    x: f32,
    y: f32,
    touch: bool,
    /// Scroll container being dragged by touch, and the last pointer position.
    scroller: Option<(Id, f32, f32)>,
    moved: bool,
}

pub struct Engine {
    pub tree: Tree,
    sheet: Sheet,
    pub media: Media,
    fonts: Fonts,
    /// Image source → (id, natural width, natural height); sizes are 0 until loaded.
    images: HashMap<String, (u32, f32, f32)>,
    image_srcs: Vec<String>,
    style_dirty: bool,
    restyle_all: bool,
    layout_dirty: bool,
    paint_dirty: bool,
    hits: Vec<Hit>,
    pub out: VecDeque<Out>,
    hover: Option<Id>,
    active: Option<Id>,
    pub focus: Option<Id>,
    pub edit: Edit,
    focus_value: String,
    popup: Option<Popup>,
    down: Option<Down>,
    last_click: Option<(f64, f32, f32, Id)>,
    selecting: bool,
    cursor: Cursor,
    blink_start: f64,
    pub now: f64,
    /// The accessibility mirror may be out of date.
    sem_dirty: bool,
    /// Running transitions by node.
    anims: HashMap<Id, Vec<Anim>>,
    /// Smooth page scroll in progress: from, to, start time.
    page_scroll: Option<(f32, f32, f64)>,
}

impl Engine {
    pub fn new(styles: &str, width: f32, height: f32) -> Engine {
        Engine {
            tree: Tree::default(),
            sheet: Sheet::new(styles),
            media: Media {
                width,
                height,
                dark: false,
                reduced_motion: false,
            },
            fonts: Fonts::default(),
            images: HashMap::new(),
            image_srcs: Vec::new(),
            style_dirty: true,
            restyle_all: true,
            layout_dirty: true,
            paint_dirty: true,
            hits: Vec::new(),
            out: VecDeque::new(),
            hover: None,
            active: None,
            focus: None,
            edit: Edit::default(),
            focus_value: String::new(),
            popup: None,
            down: None,
            last_click: None,
            selecting: false,
            cursor: Cursor::Auto,
            blink_start: 0.0,
            now: 0.0,
            sem_dirty: true,
            anims: HashMap::new(),
            page_scroll: None,
        }
    }

    /// Schedules style, layout and paint for nodes marked dirty.
    fn invalidate(&mut self) {
        self.sem_dirty = true;
        self.style_dirty = true;
        self.layout_dirty = true;
        self.paint_dirty = true;
    }

    /// Schedules a full restyle and relayout (viewport or preference changes).
    fn invalidate_all(&mut self) {
        self.tree.mark_all();
        self.restyle_all = true;
        self.invalidate();
    }

    pub fn dirty(&self) -> bool {
        self.paint_dirty
            || self.layout_dirty
            || self.style_dirty
            || !self.anims.is_empty()
            || self.page_scroll.is_some()
    }

    pub fn apply(&mut self, buf: &[u8]) -> Result<(), String> {
        let mut fx = Vec::new();
        let r = self.tree.apply(buf, &mut fx);
        for e in fx {
            self.out.push_back(match e {
                Effect::Title(s) => Out::Title(s),
                Effect::Navigate(s) => Out::Navigate(s),
                Effect::Warn(s) => Out::Warn(s),
                Effect::Fetch {
                    slot,
                    method,
                    url,
                    body,
                } => Out::Fetch {
                    slot,
                    method,
                    url,
                    body,
                },
            });
        }
        // Drop state that refers to removed nodes.
        let live = |t: &Tree, id: Option<Id>| id.filter(|&i| t.get(i).is_some());
        self.hover = live(&self.tree, self.hover);
        self.active = live(&self.tree, self.active);
        if live(&self.tree, self.focus).is_none() && self.focus.is_some() {
            self.focus = None;
            self.edit = Edit::default();
        }
        if self
            .popup
            .as_ref()
            .is_some_and(|p| self.tree.get(p.select).is_none())
        {
            self.popup = None;
        }
        if let Some(id) = self.edit.node {
            let len = self.tree.n(id).value.len();
            let v = &self.tree.n(id).value;
            let fix = |i: usize| {
                let mut i = i.min(len);
                while !v.is_char_boundary(i) {
                    i -= 1;
                }
                i
            };
            self.edit.caret = fix(self.edit.caret);
            self.edit.anchor = fix(self.edit.anchor);
        }
        self.request_images();
        self.invalidate();
        r
    }

    fn request_images(&mut self) {
        let mut want = Vec::new();
        for n in self.tree.nodes.iter().flatten() {
            if n.is("img")
                && let Some(src) = n.attr("src")
                && !self.images.contains_key(src)
            {
                want.push(src.to_string());
            }
        }
        for src in want {
            let id = self.image_srcs.len() as u32;
            self.images.insert(src.clone(), (id, 0.0, 0.0));
            self.image_srcs.push(src.clone());
            self.out.push_back(Out::LoadImage { id, url: src });
        }
    }

    pub fn image_ready(&mut self, id: u32, w: f32, h: f32) {
        if let Some(src) = self.image_srcs.get(id as usize)
            && let Some(e) = self.images.get_mut(src)
        {
            e.1 = w;
            e.2 = h;
            let src = src.clone();
            let imgs: Vec<Id> = (0..self.tree.nodes.len() as Id)
                .filter(|&i| {
                    self.tree
                        .get(i)
                        .is_some_and(|n| n.is("img") && n.attr("src") == Some(src.as_str()))
                })
                .collect();
            for i in imgs {
                self.tree.mark_layout(i);
            }
            self.invalidate();
        }
    }

    pub fn resize(&mut self, w: f32, h: f32) {
        if (w, h) != (self.media.width, self.media.height) {
            self.media.width = w;
            self.media.height = h;
            self.popup = None;
            self.invalidate_all();
        }
    }

    pub fn set_preferences(&mut self, dark: bool, reduced_motion: bool) {
        if (dark, reduced_motion) != (self.media.dark, self.media.reduced_motion) {
            self.media.dark = dark;
            self.media.reduced_motion = reduced_motion;
            self.invalidate_all();
        }
    }

    fn update_layout(&mut self, m: &mut dyn Measure) {
        if self.style_dirty {
            let mut prev = Vec::new();
            self.sheet
                .restyle_tracked(&mut self.tree, &self.media, self.restyle_all, &mut prev);
            self.start_transitions(prev);
            self.style_dirty = false;
            self.restyle_all = false;
            self.layout_dirty = true;
        }
        if self.layout_dirty {
            let sizes: HashMap<String, (f32, f32)> = self
                .images
                .iter()
                .map(|(k, v)| (k.clone(), (v.1, v.2)))
                .collect();
            let mut cx = Cx {
                t: &mut self.tree,
                m,
                fonts: &mut self.fonts,
                images: &sizes,
                vw: self.media.width,
                vh: self.media.height,
            };
            layout::layout_root(&mut cx);
            if self.sheet.uses_container_queries() {
                // Container sizes are now known; match the queries and lay out again.
                let mut prev = Vec::new();
                self.sheet
                    .restyle_tracked(&mut self.tree, &self.media, true, &mut prev);
                self.start_transitions(prev);
                let mut cx = Cx {
                    t: &mut self.tree,
                    m,
                    fonts: &mut self.fonts,
                    images: &sizes,
                    vw: self.media.width,
                    vh: self.media.height,
                };
                layout::layout_root(&mut cx);
            }
            self.layout_dirty = false;
            self.paint_dirty = true;
            self.scroll_caret_into_view(m);
        }
    }

    /// Produces a display list if anything changed since the last frame.
    pub fn frame(&mut self, m: &mut dyn Measure, now: f64) -> Option<Vec<u32>> {
        self.now = now;
        let caret_on = self.focus.is_some()
            && self.edit.node.is_some()
            && ((now - self.blink_start) / BLINK_MS) as i64 % 2 == 0;
        if caret_on != self.edit.caret_on {
            self.edit.caret_on = caret_on;
            self.paint_dirty = true;
        }
        self.update_layout(m);
        self.step_page_scroll(now);
        self.reveal();
        if !self.anims.is_empty() {
            self.paint_dirty = true;
        }
        if !self.paint_dirty {
            return None;
        }
        let saved = self.animate(now);
        let viewport = Rect {
            x: 0.0,
            y: 0.0,
            w: self.media.width,
            h: self.media.height,
        };
        let mut p = Painter::new(
            &self.tree,
            &mut self.fonts,
            m,
            &self.images,
            &self.edit,
            self.popup.as_ref(),
            viewport,
        );
        p.paint();
        let (out, hits) = (p.out, p.hits);
        for (id, s) in saved {
            self.tree.m(id).style = s;
        }
        self.hits = hits;
        self.paint_dirty = false;
        Some(out)
    }

    fn scroll_to_anchor(&mut self, id: &str) {
        let Some(target) = (0..self.tree.nodes.len() as Id)
            .find(|&i| self.tree.get(i).is_some_and(|n| n.attr("id") == Some(id)))
        else {
            return;
        };
        let max = (self.tree.n(0).lay.rect.h - self.media.height).max(0.0);
        let from = self.tree.n(0).lay.scroll.1;
        let to = clamp(from + self.abs(target).y, 0.0, max);
        if self.media.reduced_motion {
            self.tree.m(0).lay.scroll.1 = to;
            self.paint_dirty = true;
            self.sem_dirty = true;
        } else {
            self.page_scroll = Some((from, to, self.now));
        }
    }

    fn step_page_scroll(&mut self, now: f64) {
        let Some((from, to, start)) = self.page_scroll else {
            return;
        };
        let k = (((now - start) / 450.0) as f32).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - k) * (1.0 - k) * (1.0 - k);
        self.tree.m(0).lay.scroll.1 = from + (to - from) * eased;
        if k >= 1.0 {
            self.page_scroll = None;
            self.sem_dirty = true;
        }
        self.paint_dirty = true;
    }

    /// Scrolls the page back to the top (after a route change).
    pub fn scroll_to_top(&mut self) {
        self.page_scroll = None;
        self.tree.m(0).lay.scroll = (0.0, 0.0);
        self.paint_dirty = true;
        self.sem_dirty = true;
    }

    /// A key typed into a natively edited text field (only delivered as a `keydown` event).
    pub fn field_keydown(&mut self, key: &str) {
        if let Some(f) = self.focus
            && self.is_text_field(f)
        {
            self.emit(f, "keydown", Payload::Str(key.to_string()));
        }
    }

    fn start_transitions(&mut self, prev: Vec<(Id, crate::style::Style)>) {
        if self.media.reduced_motion {
            self.anims.clear();
            return;
        }
        for (id, old) in prev {
            let list = self.anims.entry(id).or_default();
            anim::start(&old, &self.tree.n(id).style, list, self.now);
            if list.is_empty() {
                self.anims.remove(&id);
            }
        }
    }

    /// Ends `when appear` styles of elements that came into view (their transitions start
    /// on the next frame).
    fn reveal(&mut self) {
        if !self.sheet.uses_appear() {
            return;
        }
        let vh = self.media.height;
        let mut shown = Vec::new();
        for id in 0..self.tree.nodes.len() as Id {
            let Some(n) = self.tree.get(id) else { continue };
            if n.flags & dom::SEEN != 0 || !n.classes.iter().any(|c| self.sheet.appears(c)) {
                continue;
            }
            let r = self.abs(id);
            if r.y < vh * 0.92 && r.y + r.h > 0.0 {
                shown.push(id);
            }
        }
        for id in shown {
            self.tree.m(id).flags |= dom::SEEN;
            self.tree.mark_style(id);
            self.invalidate();
        }
    }

    /// Writes animated values into the styles of animating nodes (and of descendants that
    /// inherit an animated color) for painting; returns the styles to restore afterwards.
    fn animate(&mut self, now: f64) -> Vec<(Id, crate::style::Style)> {
        let mut saved = Vec::new();
        let ids: Vec<Id> = self.anims.keys().copied().collect();
        for id in ids {
            if self.tree.get(id).is_none() {
                self.anims.remove(&id);
                continue;
            }
            let target = self.tree.n(id).style.color;
            saved.push((id, self.tree.n(id).style.clone()));
            let list = self.anims.get_mut(&id).unwrap();
            let s = &mut self.tree.m(id).style;
            if !anim::apply(s, list, now) {
                self.anims.remove(&id);
            }
            let color = self.tree.n(id).style.color;
            if color != target {
                let mut stack = self.tree.n(id).children.clone();
                while let Some(c) = stack.pop() {
                    if self.tree.n(c).style.color == target {
                        saved.push((c, self.tree.n(c).style.clone()));
                        self.tree.m(c).style.color = color;
                        stack.extend(self.tree.n(c).children.iter().copied());
                    }
                }
            }
        }
        // Restore in reverse so a node saved twice gets its first (target) style back.
        saved.reverse();
        saved
    }

    /// Milliseconds until the next frame is needed without input (caret blink), or -1.
    pub fn next_wake(&self, now: f64) -> f64 {
        if self.edit.node.is_none() {
            return -1.0;
        }
        let phase = (now - self.blink_start).rem_euclid(BLINK_MS);
        BLINK_MS - phase
    }

    fn restart_blink(&mut self) {
        self.blink_start = self.now;
        self.paint_dirty = true;
    }

    // ----- geometry -----

    /// Border box of `id` in viewport coordinates (as last laid out).
    pub fn abs(&self, id: Id) -> Rect {
        let n = self.tree.n(id);
        let mut r = n.lay.rect;
        if id == 0 {
            r.y -= n.lay.scroll.1;
            return r;
        }
        r.x += n.lay.offset.0;
        r.y += n.lay.offset.1;
        if n.style.position == Position::Fixed {
            return r;
        }
        let mut cur = n.parent;
        while let Some(p) = cur {
            let pn = self.tree.n(p);
            if p == 0 {
                r.y -= pn.lay.scroll.1;
                break;
            }
            r.x += pn.lay.rect.x + pn.lay.offset.0 - pn.lay.scroll.0;
            r.y += pn.lay.rect.y + pn.lay.offset.1 - pn.lay.scroll.1;
            if pn.style.position == Position::Fixed {
                break;
            }
            cur = pn.parent;
        }
        r
    }

    /// Bounding box of a text node's painted fragments, in viewport coordinates.
    fn text_rect(&self, id: Id) -> Rect {
        let rects: Vec<Rect> = self
            .hits
            .iter()
            .filter(|h| h.id == id)
            .map(|h| h.rect)
            .collect();
        let Some(first) = rects.first() else {
            return Rect::default();
        };
        let (x0, y0) = rects
            .iter()
            .fold((first.x, first.y), |(x, y), r| (x.min(r.x), y.min(r.y)));
        let (x1, y1) = rects.iter().fold((0.0f32, 0.0f32), |(x, y), r| {
            (x.max(r.x + r.w), y.max(r.y + r.h))
        });
        Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        }
    }

    fn hit(&self, x: f32, y: f32) -> Option<Id> {
        let id = paint::hit_test(&self.hits, x, y)?;
        let n = self.tree.get(id)?;
        Some(if n.kind == Kind::Text { n.parent? } else { id })
    }

    fn chain(&self, id: Option<Id>) -> Vec<Id> {
        let mut v = Vec::new();
        let mut cur = id;
        while let Some(c) = cur {
            v.push(c);
            cur = self.tree.get(c).and_then(|n| n.parent);
        }
        v
    }

    fn set_flag(&mut self, old: Option<Id>, new: Option<Id>, flag: u8) {
        if old == new {
            return;
        }
        for c in self.chain(old) {
            self.tree.m(c).flags &= !flag;
            self.tree.mark_style(c);
        }
        for c in self.chain(new) {
            self.tree.m(c).flags |= flag;
            self.tree.mark_style(c);
        }
        self.invalidate();
    }

    // ----- focus -----

    fn disabled(&self, id: Id) -> bool {
        self.tree.n(id).attr("disabled").is_some()
    }

    fn focusable(&self, id: Id) -> bool {
        let n = self.tree.n(id);
        if n.kind != Kind::Element
            || self.disabled(id)
            || n.style.display == crate::style::Display::None
        {
            return false;
        }
        match n.tag.as_str() {
            "button" | "input" | "textarea" | "select" => true,
            "a" => n.attr("href").is_some(),
            _ => false,
        }
    }

    fn is_text_field(&self, id: Id) -> bool {
        let n = self.tree.n(id);
        n.is("textarea") || (n.is("input") && n.attr("type") != Some("checkbox"))
    }

    /// Moves focus, emitting `change`/`blur` on the old element and `focus` on the new.
    fn set_focus(&mut self, new: Option<Id>, visible: bool) {
        if new == self.focus {
            if let Some(f) = new
                && visible != (self.tree.n(f).flags & dom::FOCUS_VISIBLE != 0)
            {
                self.tree.m(f).flags ^= dom::FOCUS_VISIBLE;
                self.tree.mark_style(f);
                self.invalidate();
            }
            return;
        }
        if let Some(old) = self.focus {
            self.commit_change(old);
            self.emit(old, "blur", Payload::None);
            self.tree.m(old).flags &= !(dom::FOCUS | dom::FOCUS_VISIBLE);
            for c in self.chain(Some(old)) {
                self.tree.m(c).flags &= !dom::FOCUS_WITHIN;
                self.tree.mark_style(c);
            }
        }
        self.popup = None;
        self.focus = new;
        self.edit = Edit::default();
        if let Some(id) = new {
            let field = self.is_text_field(id);
            self.tree.m(id).flags |= dom::FOCUS
                | if visible || field {
                    dom::FOCUS_VISIBLE
                } else {
                    0
                };
            for c in self.chain(Some(id)) {
                if c != id {
                    self.tree.m(c).flags |= dom::FOCUS_WITHIN;
                }
                self.tree.mark_style(c);
            }
            self.focus_value = self.tree.n(id).value.clone();
            if field {
                let len = self.tree.n(id).value.len();
                self.edit = Edit {
                    node: Some(id),
                    caret: len,
                    anchor: len,
                    ..Edit::default()
                };
                self.restart_blink();
            }
            self.emit(id, "focus", Payload::None);
        }
        self.invalidate();
    }

    fn commit_change(&mut self, id: Id) {
        if self.is_text_field(id) && self.tree.n(id).value != self.focus_value {
            self.focus_value = self.tree.n(id).value.clone();
            let v = self.focus_value.clone();
            self.emit(id, "change", Payload::Str(v));
        }
    }

    fn focus_order(&self) -> Vec<Id> {
        let mut v = Vec::new();
        let mut stack = vec![0];
        while let Some(id) = stack.pop() {
            if self.tree.n(id).style.display == crate::style::Display::None {
                continue;
            }
            if self.focusable(id) {
                v.push(id);
            }
            stack.extend(self.tree.n(id).children.iter().rev());
        }
        v
    }

    /// Moves focus to the next or previous element; false when focus leaves the page.
    fn tab(&mut self, backward: bool) -> bool {
        let order = self.focus_order();
        let pos = self.focus.and_then(|f| order.iter().position(|&x| x == f));
        let next = match (pos, backward) {
            (None, false) => order.first().copied(),
            (None, true) => order.last().copied(),
            (Some(i), false) => order.get(i + 1).copied(),
            (Some(i), true) => i.checked_sub(1).map(|j| order[j]),
        };
        match next {
            Some(id) => {
                self.set_focus(Some(id), true);
                self.scroll_into_view(id);
                true
            }
            None => {
                self.set_focus(None, false);
                false
            }
        }
    }

    /// Called when keyboard focus enters the canvas from outside the page.
    pub fn enter(&mut self, backward: bool) {
        if self.focus.is_none() {
            self.tab(backward);
        }
    }

    /// Called when the canvas loses focus.
    pub fn leave(&mut self) {
        self.set_focus(None, false);
        self.down = None;
    }

    fn scroll_into_view(&mut self, id: Id) {
        let r = self.abs(id);
        let vh = self.media.height;
        let root = &mut self.tree.m(0).lay;
        let max = (root.rect.h - vh).max(0.0);
        if r.y < 0.0 {
            root.scroll.1 = clamp(root.scroll.1 + r.y - 8.0, 0.0, max);
        } else if r.y + r.h > vh {
            root.scroll.1 = clamp(root.scroll.1 + r.y + r.h - vh + 8.0, 0.0, max);
        }
        self.paint_dirty = true;
        self.sem_dirty = true;
    }

    // ----- events -----

    fn emit(&mut self, id: Id, event: &str, payload: Payload) {
        if let Some(l) = self.tree.n(id).listener(event) {
            self.out.push_back(Out::Emit {
                node: id,
                slot: l.slot,
                payload,
            });
        }
    }

    /// Emits `event` on `id` and each ancestor that listens for it.
    fn bubble(&mut self, id: Id, event: &str) {
        for c in self.chain(Some(id)) {
            self.emit(c, event, Payload::None);
        }
    }

    fn closest(&self, id: Id, pred: impl Fn(&dom::Node) -> bool) -> Option<Id> {
        self.chain(Some(id))
            .into_iter()
            .find(|&c| pred(self.tree.n(c)))
    }

    fn checkbox_of(&self, id: Id) -> Option<Id> {
        let label = self.closest(id, |n| {
            n.has_class("v-check") || (n.is("input") && n.attr("type") == Some("checkbox"))
        })?;
        let n = self.tree.n(label);
        if n.is("input") {
            Some(label)
        } else {
            n.children
                .iter()
                .copied()
                .find(|&c| self.tree.n(c).is("input"))
        }
    }

    fn update_hover(&mut self, x: f32, y: f32) {
        let target = self.hit(x, y);
        let old = self.hover;
        self.hover = target;
        self.set_flag(old, target, dom::HOVER);
        let cursor = match target {
            Some(t) => {
                let s = &self.tree.n(t).style;
                if s.cursor == Cursor::Auto
                    && self.closest(t, |n| n.attr("disabled").is_some()).is_some()
                {
                    Cursor::Default
                } else {
                    s.cursor
                }
            }
            None => Cursor::Auto,
        };
        if self.popup.as_ref().is_some_and(|p| p.rect.contains(x, y)) {
            let p = self.popup.as_mut().unwrap();
            let i = ((y - p.rect.y - 1.0) / p.item_h).floor().max(0.0) as usize;
            if i != p.hover {
                p.hover = i;
                self.paint_dirty = true;
            }
        }
        if cursor != self.cursor {
            self.cursor = cursor;
            self.out.push_back(Out::Cursor(match cursor {
                Cursor::Auto | Cursor::Default => "default",
                Cursor::Pointer => "pointer",
                Cursor::Text => "text",
                Cursor::NotAllowed => "not-allowed",
                Cursor::Grab => "grab",
                Cursor::Move => "move",
            }));
        }
    }

    pub fn pointer_down(&mut self, m: &mut dyn Measure, x: f32, y: f32, touch: bool) {
        self.update_layout(m);
        if let Some(p) = self.popup.clone() {
            if p.rect.contains(x, y) {
                return;
            }
            self.popup = None;
            self.paint_dirty = true;
            if self.hit(x, y) == Some(p.select) {
                return;
            }
        }
        let Some(target) = self.hit(x, y) else { return };
        let scroller = if touch {
            self.scroller_at(x, y).map(|s| (s, x, y))
        } else {
            None
        };
        self.down = Some(Down {
            target,
            x,
            y,
            touch,
            scroller,
            moved: false,
        });
        if !touch {
            self.press(target, x, y, m);
        }
    }

    fn press(&mut self, target: Id, x: f32, y: f32, m: &mut dyn Measure) {
        let old = self.active;
        self.active = Some(target);
        self.set_flag(old, Some(target), dom::ACTIVE);
        if self
            .closest(target, |n| n.attr("disabled").is_some())
            .is_some()
        {
            return;
        }
        let focus = self.checkbox_of(target).or_else(|| {
            self.chain(Some(target))
                .into_iter()
                .find(|&c| self.focusable(c))
        });
        self.set_focus(focus, false);
        if let Some(f) = focus
            && self.is_text_field(f)
        {
            let at = self.offset_at(m, f, x, y);
            self.edit.caret = at;
            self.edit.anchor = at;
            self.selecting = true;
            self.restart_blink();
        }
    }

    pub fn pointer_move(&mut self, m: &mut dyn Measure, x: f32, y: f32) {
        if let Some(d) = &mut self.down
            && d.touch
        {
            if (x - d.x).abs() + (y - d.y).abs() > 10.0 {
                d.moved = true;
            }
            if let Some((s, lx, ly)) = d.scroller {
                d.scroller = Some((s, x, y));
                self.scroll_by(s, lx - x, ly - y);
            }
            return;
        }
        self.update_hover(x, y);
        if self.selecting
            && let Some(f) = self.edit.node
        {
            self.update_layout(m);
            let at = self.offset_at(m, f, x, y);
            if at != self.edit.caret {
                self.edit.caret = at;
                self.scroll_caret_into_view(m);
                self.paint_dirty = true;
            }
        }
    }

    pub fn pointer_up(&mut self, m: &mut dyn Measure, x: f32, y: f32) {
        self.selecting = false;
        let Some(down) = self.down.take() else {
            if let Some(p) = self.popup.clone()
                && p.rect.contains(x, y)
            {
                let i = ((y - p.rect.y - 1.0) / p.item_h).floor().max(0.0) as usize;
                self.choose(p.select, i);
                self.popup = None;
                self.paint_dirty = true;
            }
            return;
        };
        if down.touch {
            if down.moved {
                return;
            }
            self.press(down.target, x, y, m);
        }
        let old = self.active.take();
        self.set_flag(old, None, dom::ACTIVE);
        let Some(up) = self.hit(x, y) else { return };
        // The click goes to the deepest element containing both ends.
        let downs = self.chain(Some(down.target));
        let Some(target) = self.chain(Some(up)).into_iter().find(|c| downs.contains(c)) else {
            return;
        };
        if self
            .closest(target, |n| n.attr("disabled").is_some())
            .is_some()
        {
            return;
        }
        self.bubble(target, "click");
        let double = self.last_click.is_some_and(|(t, lx, ly, id)| {
            id == target
                && self.now - t < DOUBLE_CLICK_MS
                && (lx - x).abs() < 5.0
                && (ly - y).abs() < 5.0
        });
        if double {
            self.bubble(target, "dblclick");
            self.last_click = None;
        } else {
            self.last_click = Some((self.now, x, y, target));
        }
        self.activate(target);
    }

    /// Default action of a click or keyboard activation.
    fn activate(&mut self, target: Id) {
        if let Some(cb) = self.checkbox_of(target) {
            if !self.disabled(cb) {
                let on = !self.tree.n(cb).checked;
                self.tree.m(cb).checked = on;
                self.tree.mark_style(cb);
                if let Some(label) = self.tree.n(cb).parent {
                    self.tree.mark_style(label);
                }
                self.emit(cb, "change", Payload::Bool(on));
                self.invalidate();
            }
            return;
        }
        for c in self.chain(Some(target)) {
            let n = self.tree.n(c);
            match n.tag.as_str() {
                "button" => {
                    if n.attr("type") == Some("submit") {
                        self.submit(c);
                    }
                    return;
                }
                "a" => {
                    if let Some(href) = n.attr("href") {
                        let href = href.to_string();
                        self.follow(&href, false);
                    }
                    return;
                }
                "select" => {
                    self.open_popup(c);
                    return;
                }
                _ => {}
            }
        }
    }

    fn follow(&mut self, href: &str, new_tab: bool) {
        match href.strip_prefix('#') {
            // `#name` scrolls to the element with that id; `#/path` is a route.
            Some(id) if !new_tab && !id.starts_with('/') => self.scroll_to_anchor(id),
            Some(path) if !new_tab => self.out.push_back(Out::Navigate(path.to_string())),
            _ => self.out.push_back(Out::Open {
                url: href.to_string(),
                new_tab,
            }),
        }
    }

    fn submit(&mut self, from: Id) {
        if let Some(form) = self.closest(from, |n| n.is("form")) {
            if let Some(f) = self.focus {
                self.commit_change(f);
            }
            self.emit(form, "submit", Payload::None);
        }
    }

    fn open_popup(&mut self, select: Id) {
        let n = self.tree.n(select);
        let count = n.children.len();
        if count == 0 {
            return;
        }
        let current = n
            .children
            .iter()
            .position(|&o| self.tree.n(o).attr("value") == Some(n.value.as_str()))
            .unwrap_or(0);
        let item_h = n.style.line_height + 8.0;
        let r = self.abs(select);
        let width = r.w.max(120.0);
        let h = count as f32 * item_h + 2.0;
        let below = r.y + r.h + h <= self.media.height || r.y < h;
        let y = if below {
            r.y + r.h + 2.0
        } else {
            r.y - h - 2.0
        };
        self.popup = Some(Popup {
            select,
            hover: current,
            rect: Rect {
                x: r.x,
                y,
                w: width,
                h,
            },
            item_h,
        });
        self.paint_dirty = true;
    }

    fn choose(&mut self, select: Id, i: usize) {
        let Some(&opt) = self.tree.n(select).children.get(i) else {
            return;
        };
        let value = self.tree.n(opt).attr("value").unwrap_or("").to_string();
        if self.tree.n(select).value != value {
            self.tree.m(select).value = value.clone();
            self.emit(select, "change", Payload::Str(value));
            self.invalidate();
        }
    }

    // ----- scrolling -----

    fn can_scroll(&self, id: Id, dx: f32, dy: f32) -> bool {
        let n = self.tree.n(id);
        let (mx, my) = n.lay.max_scroll();
        let (sx, sy) = n.lay.scroll;
        (dy > 0.0 && sy < my && n.style.overflow_y == Overflow::Scroll)
            || (dy < 0.0 && sy > 0.0 && n.style.overflow_y == Overflow::Scroll)
            || (dx > 0.0 && sx < mx && n.style.overflow_x == Overflow::Scroll)
            || (dx < 0.0 && sx > 0.0 && n.style.overflow_x == Overflow::Scroll)
    }

    fn scroller_at(&self, x: f32, y: f32) -> Option<Id> {
        let start = self.hit(x, y)?;
        self.chain(Some(start))
            .into_iter()
            .find(|&c| {
                c != 0
                    && self.tree.n(c).style.clips()
                    && self.tree.n(c).lay.max_scroll() != (0.0, 0.0)
            })
            .or(Some(0))
    }

    fn scroll_by(&mut self, id: Id, dx: f32, dy: f32) {
        if id == 0 {
            let vh = self.media.height;
            let root = &mut self.tree.m(0).lay;
            let max = (root.rect.h - vh).max(0.0);
            root.scroll.1 = clamp(root.scroll.1 + dy, 0.0, max);
        } else {
            let lay = &mut self.tree.m(id).lay;
            let (mx, my) = lay.max_scroll();
            lay.scroll = (
                clamp(lay.scroll.0 + dx, 0.0, mx),
                clamp(lay.scroll.1 + dy, 0.0, my),
            );
        }
        self.popup = None;
        self.paint_dirty = true;
        self.sem_dirty = true;
    }

    pub fn wheel(&mut self, x: f32, y: f32, dx: f32, dy: f32) {
        let start = self.hit(x, y);
        let target = self
            .chain(start)
            .into_iter()
            .find(|&c| c != 0 && self.can_scroll(c, dx, dy))
            .unwrap_or(0);
        self.scroll_by(target, dx, dy);
        self.update_hover(x, y);
    }

    // ----- text editing -----

    /// Content box of a field in viewport coordinates.
    fn field_box(&self, id: Id) -> Rect {
        let r = self.abs(id);
        let c = self.tree.n(id).lay.content_box();
        Rect {
            x: r.x + c.x,
            y: r.y + c.y,
            w: c.w,
            h: c.h,
        }
    }

    fn lines(&mut self, m: &mut dyn Measure, id: Id) -> Vec<(usize, usize)> {
        let n = self.tree.n(id);
        if n.is("textarea") {
            let w = n.lay.content_box().w;
            layout::wrap_lines(&mut self.fonts, m, &n.style.clone(), &n.value.clone(), w)
        } else {
            vec![(0, n.value.len())]
        }
    }

    /// Byte offset in a field's value nearest to a viewport point.
    fn offset_at(&mut self, m: &mut dyn Measure, id: Id, x: f32, y: f32) -> usize {
        let content = self.field_box(id);
        let lines = self.lines(m, id);
        let n = self.tree.n(id);
        let s = n.style.clone();
        let value = n.value.clone();
        let line = (((y - content.y + self.edit.scroll.1) / s.line_height)
            .floor()
            .max(0.0) as usize)
            .min(lines.len() - 1);
        let (a, z) = lines[line];
        let target = x - content.x + self.edit.scroll.0;
        let mut best = (a, f32::MAX);
        for (i, _) in value[a..z]
            .char_indices()
            .map(|(i, c)| (a + i, c))
            .chain(std::iter::once((z, ' ')))
        {
            let w = self.fonts.width(m, &s, &value[a..i]);
            let d = (w - target).abs();
            if d < best.1 {
                best = (i, d);
            }
        }
        best.0
    }

    fn scroll_caret_into_view(&mut self, m: &mut dyn Measure) {
        let Some(id) = self.edit.node else { return };
        if self.tree.get(id).is_none() {
            return;
        }
        let lines = self.lines(m, id);
        let n = self.tree.n(id);
        let s = n.style.clone();
        let value = n.value.clone();
        let c = n.lay.content_box();
        let (line, x) = paint::caret_xy(&mut self.fonts, m, &s, &value, &lines, self.edit.caret);
        let sc = &mut self.edit.scroll;
        if x - sc.0 > c.w - 1.0 {
            sc.0 = x - c.w + 1.0;
        }
        if x < sc.0 {
            sc.0 = x;
        }
        let top = line as f32 * s.line_height;
        if top + s.line_height - sc.1 > c.h {
            sc.1 = top + s.line_height - c.h;
        }
        if top < sc.1 {
            sc.1 = top;
        }
    }

    fn replace_selection(&mut self, id: Id, text: &str) {
        let (a, z) = self.edit.range();
        let v = &mut self.tree.m(id).value;
        v.replace_range(a..z, text);
        self.edit.caret = a + text.len();
        self.edit.anchor = self.edit.caret;
        let value = v.clone();
        self.emit(id, "input", Payload::Str(value));
        self.invalidate();
        self.restart_blink();
    }

    /// Inserts pasted or typed text into the focused field.
    pub fn insert_text(&mut self, text: &str) {
        let Some(id) = self.edit.node else { return };
        if self.disabled(id) {
            return;
        }
        let text = if self.tree.n(id).is("input") {
            text.replace(['\n', '\r'], " ")
        } else {
            text.replace('\r', "")
        };
        self.replace_selection(id, &text);
    }

    fn move_caret(&mut self, m: &mut dyn Measure, id: Id, key: &str, shift: bool, word: bool) {
        let value = self.tree.n(id).value.clone();
        let (a, z) = self.edit.range();
        let prev = |i: usize| {
            value[..i]
                .chars()
                .next_back()
                .map_or(0, |c| i - c.len_utf8())
        };
        let next = |i: usize| {
            value[i..]
                .chars()
                .next()
                .map_or(value.len(), |c| i + c.len_utf8())
        };
        let lines = self.lines(m, id);
        let caret = self.edit.caret;
        let to = match key {
            "ArrowLeft" if !shift && a != z => a,
            "ArrowRight" if !shift && a != z => z,
            "ArrowLeft" if word => value[..caret]
                .trim_end()
                .rfind(char::is_whitespace)
                .map_or(0, |i| i + 1),
            "ArrowRight" if word => {
                let rest = &value[caret..];
                let skip = rest.len() - rest.trim_start().len();
                rest[skip..]
                    .find(char::is_whitespace)
                    .map_or(value.len(), |i| caret + skip + i)
            }
            "ArrowLeft" => prev(caret),
            "ArrowRight" => next(caret),
            "Home" => lines.iter().rev().find(|l| l.0 <= caret).map_or(0, |l| l.0),
            "End" => lines
                .iter()
                .find(|l| l.1 >= caret)
                .map_or(value.len(), |l| l.1),
            "ArrowUp" | "ArrowDown" => {
                let s = self.tree.n(id).style.clone();
                let (line, x) = paint::caret_xy(&mut self.fonts, m, &s, &value, &lines, caret);
                let target = if key == "ArrowUp" {
                    line.checked_sub(1)
                } else {
                    Some(line + 1).filter(|&l| l < lines.len())
                };
                match target {
                    None if key == "ArrowUp" => 0,
                    None => value.len(),
                    Some(l) => {
                        let (la, lz) = lines[l];
                        let mut best = (la, f32::MAX);
                        for i in value[la..lz]
                            .char_indices()
                            .map(|(i, _)| la + i)
                            .chain(std::iter::once(lz))
                        {
                            let d = (self.fonts.width(m, &s, &value[la..i]) - x).abs();
                            if d < best.1 {
                                best = (i, d);
                            }
                        }
                        best.0
                    }
                }
            }
            _ => caret,
        };
        self.edit.caret = to;
        if !shift {
            self.edit.anchor = to;
        }
        self.restart_blink();
        self.scroll_caret_into_view(m);
    }

    fn field_key(
        &mut self,
        m: &mut dyn Measure,
        id: Id,
        key: &str,
        shift: bool,
        ctrl: bool,
    ) -> bool {
        let textarea = self.tree.n(id).is("textarea");
        let value = self.tree.n(id).value.clone();
        let (a, z) = self.edit.range();
        match key {
            "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" | "Home" | "End" => {
                self.move_caret(m, id, key, shift, ctrl)
            }
            "Backspace" | "Delete" => {
                if a == z {
                    let caret = self.edit.caret;
                    if key == "Backspace" {
                        if caret == 0 {
                            return true;
                        }
                        let start = if ctrl {
                            value[..caret]
                                .trim_end()
                                .rfind(char::is_whitespace)
                                .map_or(0, |i| i + 1)
                        } else {
                            value[..caret]
                                .chars()
                                .next_back()
                                .map_or(0, |c| caret - c.len_utf8())
                        };
                        self.edit.anchor = start;
                    } else {
                        if caret == value.len() {
                            return true;
                        }
                        self.edit.anchor = value[caret..]
                            .chars()
                            .next()
                            .map_or(caret, |c| caret + c.len_utf8());
                    }
                }
                self.replace_selection(id, "");
            }
            "Enter" if textarea => self.replace_selection(id, "\n"),
            "Enter" => {
                self.commit_change(id);
                self.submit(id);
            }
            "a" if ctrl => {
                self.edit.anchor = 0;
                self.edit.caret = value.len();
                self.paint_dirty = true;
            }
            "c" | "x" if ctrl => {
                if a != z {
                    self.out.push_back(Out::Copy(value[a..z].to_string()));
                    if key == "x" {
                        self.replace_selection(id, "");
                    }
                }
            }
            // Paste arrives through `insert_text`; let the browser produce the paste event.
            "v" if ctrl => return false,
            _ if ctrl => return false,
            k if k.chars().count() == 1 => self.insert_text(k),
            _ => return false,
        }
        self.paint_dirty = true;
        true
    }

    /// Handles a key press; returns true if the page consumed it.
    pub fn key(
        &mut self,
        m: &mut dyn Measure,
        key: &str,
        shift: bool,
        ctrl: bool,
        alt: bool,
    ) -> bool {
        self.update_layout(m);
        if let Some(p) = self.popup.clone() {
            let count = self.tree.n(p.select).children.len();
            match key {
                "ArrowDown" => self.popup.as_mut().unwrap().hover = (p.hover + 1).min(count - 1),
                "ArrowUp" => self.popup.as_mut().unwrap().hover = p.hover.saturating_sub(1),
                "Enter" | " " => {
                    self.choose(p.select, p.hover);
                    self.popup = None;
                }
                "Escape" | "Tab" => self.popup = None,
                _ => return true,
            }
            self.paint_dirty = true;
            return key != "Tab" || self.tab(shift);
        }
        if key == "Tab" {
            return self.tab(shift);
        }
        let Some(f) = self.focus else {
            return self.page_key(key, shift);
        };
        if self.is_text_field(f) {
            self.emit(f, "keydown", Payload::Str(key.to_string()));
            return self.field_key(m, f, key, shift, ctrl);
        }
        let n = self.tree.n(f);
        match (n.tag.as_str(), key) {
            ("input", " ") => self.activate(f),
            ("button", "Enter" | " ") | ("a", "Enter") => {
                self.bubble(f, "click");
                self.activate(f);
            }
            ("select", "Enter" | " ") => self.open_popup(f),
            ("select", "ArrowDown") if alt => self.open_popup(f),
            ("select", "ArrowDown" | "ArrowUp") => {
                let i = n
                    .children
                    .iter()
                    .position(|&o| self.tree.n(o).attr("value") == Some(n.value.as_str()))
                    .unwrap_or(0);
                let j = if key == "ArrowDown" {
                    (i + 1).min(n.children.len().saturating_sub(1))
                } else {
                    i.saturating_sub(1)
                };
                self.choose(f, j);
            }
            ("button" | "input", "Escape") => self.set_focus(None, false),
            _ => return self.page_key(key, shift),
        }
        true
    }

    fn page_key(&mut self, key: &str, shift: bool) -> bool {
        let page = self.media.height * 0.9;
        let dy = match key {
            "ArrowDown" => LINE_SCROLL,
            "ArrowUp" => -LINE_SCROLL,
            "PageDown" => page,
            "PageUp" => -page,
            " " if shift => -page,
            " " => page,
            "Home" => -1e9,
            "End" => 1e9,
            _ => return false,
        };
        let target = self
            .focus
            .or(self.hover)
            .and_then(|s| {
                self.chain(Some(s))
                    .into_iter()
                    .find(|&c| c != 0 && self.can_scroll(c, 0.0, dy))
            })
            .unwrap_or(0);
        self.scroll_by(target, 0.0, dy);
        true
    }

    // ----- host interface -----

    /// Next batch of work for the host: effects up to and including one event. Events whose
    /// element or listener is gone by the time they are delivered are dropped.
    pub fn poll(&mut self) -> Vec<u8> {
        let mut w = Writer(Vec::new());
        while let Some(o) = self.out.pop_front() {
            match o {
                Out::Emit {
                    node,
                    slot,
                    payload,
                } => {
                    let alive = self
                        .tree
                        .get(node)
                        .is_some_and(|n| n.listeners.iter().any(|l| l.slot == slot));
                    if !alive {
                        continue;
                    }
                    w.int(out::EMIT).int(slot);
                    match payload {
                        Payload::None => w.int(0),
                        Payload::Str(s) => w.int(1).str(&s),
                        Payload::Bool(b) => w.int(2).int(b as i32),
                    };
                    break;
                }
                Out::Title(s) => w.int(out::TITLE).str(&s),
                Out::Navigate(s) => w.int(out::NAVIGATE).str(&s),
                Out::Open { url, new_tab } => w.int(out::OPEN).str(&url).int(new_tab as i32),
                Out::Warn(s) => w.int(out::WARN).str(&s),
                Out::Fetch {
                    slot,
                    method,
                    url,
                    body,
                } => w
                    .int(out::FETCH)
                    .int(slot)
                    .int(method)
                    .int(body.is_some() as i32)
                    .str(&url)
                    .str(body.as_deref().unwrap_or("")),
                Out::LoadImage { id, url } => w.int(out::LOAD_IMAGE).int(id as i32).str(&url),
                Out::Cursor(c) => w.int(out::CURSOR).str(c),
                Out::Copy(s) => w.int(out::COPY).str(&s),
            };
        }
        w.0
    }

    /// The tree as JSON, with viewport rectangles, for tests and debugging.
    pub fn dump(&self) -> String {
        let mut s = String::from("[");
        let mut stack = vec![0];
        let mut first = true;
        while let Some(id) = stack.pop() {
            let n = self.tree.n(id);
            if !first {
                s.push(',');
            }
            first = false;
            let r = if n.kind == Kind::Text {
                self.text_rect(id)
            } else {
                self.abs(id)
            };
            let shown = self.hits.iter().any(|h| h.id == id);
            s.push_str(&format!(
                "{{\"id\":{id},\"parent\":{},",
                n.parent.map_or(-1, |p| p as i64)
            ));
            match n.kind {
                Kind::Text => {
                    s.push_str("\"text\":");
                    json_str(&mut s, &n.text);
                }
                Kind::Comment => s.push_str("\"comment\":true"),
                Kind::Element => {
                    s.push_str("\"tag\":");
                    json_str(&mut s, &n.tag);
                    s.push_str(",\"classes\":[");
                    for (i, c) in n.classes.iter().enumerate() {
                        if i > 0 {
                            s.push(',');
                        }
                        json_str(&mut s, c);
                    }
                    s.push_str("],\"attrs\":{");
                    for (i, (k, v)) in n.attrs.iter().enumerate() {
                        if i > 0 {
                            s.push(',');
                        }
                        json_str(&mut s, k);
                        s.push(':');
                        json_str(&mut s, v);
                    }
                    s.push_str("},\"value\":");
                    json_str(&mut s, &n.value);
                    s.push_str(&format!(
                        ",\"checked\":{},\"focused\":{},\"listeners\":{},\"color\":\"#{:08x}\",\"background\":\"{}\",\"decoration\":\"{:?}\"",
                        n.checked,
                        self.focus == Some(id),
                        n.listeners.len(),
                        n.style.color,
                        match &n.style.background {
                            crate::style::Background::Color(c) => format!("#{c:08x}"),
                            crate::style::Background::Linear(..) => "gradient".into(),
                            crate::style::Background::None => String::new(),
                        },
                        n.style.decoration
                    ));
                }
            }
            s.push_str(&format!(
                ",\"rect\":[{},{},{},{}],\"shown\":{shown}}}",
                num(r.x),
                num(r.y),
                num(r.w),
                num(r.h)
            ));
            stack.extend(n.children.iter().rev());
        }
        s.push(']');
        s
    }
}

fn json_str(s: &mut String, v: &str) {
    s.push('"');
    for c in v.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            c if (c as u32) < 0x20 => s.push_str(&format!("\\u{:04x}", c as u32)),
            c => s.push(c),
        }
    }
    s.push('"');
}

struct Writer(Vec<u8>);

impl Writer {
    fn int(&mut self, v: i32) -> &mut Self {
        self.0.extend(v.to_le_bytes());
        self
    }

    fn rect(&mut self, r: Rect) -> &mut Self {
        for v in [r.x, r.y, r.w, r.h] {
            self.0.extend(v.to_le_bytes());
        }
        self
    }

    fn str(&mut self, s: &str) -> &mut Self {
        self.int(s.len() as i32);
        self.0.extend(s.as_bytes());
        while !self.0.len().is_multiple_of(4) {
            self.0.push(0);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::tests::Buf;
    use crate::text::tests::Mono;

    fn engine(b: &Buf, css: &str) -> Engine {
        let mut e = Engine::new(css, 800.0, 600.0);
        e.apply(&b.0).unwrap();
        e.frame(&mut Mono, 0.0).unwrap();
        e.out.clear();
        e
    }

    fn center(e: &Engine, id: Id) -> (f32, f32) {
        let r = e.abs(id);
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    fn click(e: &mut Engine, id: Id) {
        let (x, y) = center(e, id);
        e.pointer_down(&mut Mono, x, y, false);
        e.pointer_up(&mut Mono, x, y);
        e.frame(&mut Mono, e.now);
    }

    fn emits(e: &mut Engine) -> Vec<(i32, Payload)> {
        e.out
            .drain(..)
            .filter_map(|o| {
                if let Out::Emit { slot, payload, .. } = o {
                    Some((slot, payload))
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn anchors_scroll_and_appear_styles_transition_in() {
        let mut b = Buf::default();
        b.create(1, "div").class(1, "tall").insert(0, 1, 0);
        b.create(2, "div")
            .class(2, "fade")
            .attr(2, "id", "below")
            .insert(0, 2, 0);
        b.create(3, "div").class(3, "tall").insert(0, 3, 0);
        let css = ".tall{height:2000px}.fade{height:10px;transition:opacity 100ms linear}.fade:appear{opacity:0}";
        let mut e = engine(&b, css);
        assert_eq!(e.tree.n(2).style.opacity, 0.0, "below the fold");
        e.follow("#below", false);
        assert!(e.out.is_empty(), "an anchor is not a route");
        e.frame(&mut Mono, 1000.0);
        assert!(e.abs(2).y.abs() < 1.0, "scrolled to the anchor");
        e.frame(&mut Mono, 1000.0);
        assert_eq!(e.tree.n(2).style.opacity, 1.0);
        let list = e.frame(&mut Mono, 1050.0).unwrap();
        let half = list
            .windows(2)
            .any(|w| w[0] == paint::op::ALPHA && f32::from_bits(w[1]) == 0.5);
        assert!(half, "painted halfway through the transition");
        e.frame(&mut Mono, 2000.0);
        assert!(!e.dirty(), "the transition finished");
    }

    #[test]
    fn click_bubbles_through_listeners() {
        let mut b = Buf::default();
        b.create(1, "div").listen(1, 10, "click").insert(0, 1, 0);
        b.create(2, "button")
            .listen(2, 11, "click")
            .insert(1, 2, 0)
            .text(3, "Go")
            .insert(2, 3, 0);
        let mut e = engine(&b, "");
        click(&mut e, 2);
        assert_eq!(
            emits(&mut e),
            vec![(11, Payload::None), (10, Payload::None)]
        );
        assert_eq!(e.focus, Some(2));
        // Mouse focus on a button is not focus-visible.
        assert_eq!(e.tree.n(2).flags & dom::FOCUS_VISIBLE, 0);
    }

    #[test]
    fn disabled_buttons_ignore_clicks() {
        let mut b = Buf::default();
        b.create(1, "button")
            .attr(1, "disabled", "")
            .listen(1, 5, "click")
            .insert(0, 1, 0)
            .text(2, "x")
            .insert(1, 2, 0);
        let mut e = engine(&b, "");
        click(&mut e, 1);
        assert!(emits(&mut e).is_empty());
    }

    #[test]
    fn typing_in_a_field() {
        let mut b = Buf::default();
        b.create(1, "form")
            .listen(1, 1, "submit:prevent")
            .insert(0, 1, 0);
        b.create(2, "input")
            .listen(2, 2, "input:value")
            .listen(2, 3, "change:value")
            .listen(2, 4, "keydown:key")
            .insert(1, 2, 0);
        let mut e = engine(&b, "");
        click(&mut e, 2);
        assert_eq!(e.focus, Some(2));
        for k in ["h", "i", "!", "Backspace"] {
            assert!(e.key(&mut Mono, k, false, false, false));
        }
        assert_eq!(e.tree.n(2).value, "hi");
        let ev = emits(&mut e);
        assert_eq!(ev[0], (4, Payload::Str("h".into())));
        assert_eq!(ev[1], (2, Payload::Str("h".into())));
        assert_eq!(ev.last().unwrap(), &(2, Payload::Str("hi".into())));
        e.key(&mut Mono, "ArrowLeft", false, false, false);
        e.insert_text("é");
        assert_eq!(e.tree.n(2).value, "héi");
        e.key(&mut Mono, "a", false, true, false);
        e.key(&mut Mono, "c", false, true, false);
        assert!(e.out.contains(&Out::Copy("héi".into())));
        emits(&mut e);
        e.key(&mut Mono, "Enter", false, false, false);
        assert_eq!(
            emits(&mut e),
            vec![
                (4, Payload::Str("Enter".into())),
                (3, Payload::Str("héi".into())),
                (1, Payload::None)
            ]
        );
    }

    #[test]
    fn checkbox_label_toggles() {
        let mut b = Buf::default();
        b.create(1, "label").class(1, "v-check").insert(0, 1, 0);
        b.create(2, "input")
            .attr(2, "type", "checkbox")
            .listen(2, 9, "change:checked")
            .insert(1, 2, 0);
        b.text(3, "Done").insert(1, 3, 0);
        let mut e = engine(&b, "");
        click(&mut e, 1);
        assert!(e.tree.n(2).checked);
        assert_eq!(emits(&mut e), vec![(9, Payload::Bool(true))]);
        e.key(&mut Mono, " ", false, false, false);
        assert!(!e.tree.n(2).checked);
    }

    #[test]
    fn tab_order_and_links() {
        let mut b = Buf::default();
        b.create(1, "button")
            .insert(0, 1, 0)
            .text(2, "a")
            .insert(1, 2, 0);
        b.create(3, "a")
            .attr(3, "href", "#/about")
            .insert(0, 3, 0)
            .text(4, "About")
            .insert(3, 4, 0);
        b.create(5, "button")
            .attr(5, "disabled", "")
            .insert(0, 5, 0)
            .text(6, "c")
            .insert(5, 6, 0);
        let mut e = engine(&b, "");
        assert!(e.key(&mut Mono, "Tab", false, false, false));
        assert_eq!(e.focus, Some(1));
        assert_ne!(e.tree.n(1).flags & dom::FOCUS_VISIBLE, 0);
        e.key(&mut Mono, "Tab", false, false, false);
        assert_eq!(e.focus, Some(3));
        e.key(&mut Mono, "Enter", false, false, false);
        assert!(e.out.contains(&Out::Navigate("/about".into())));
        // Past the last focusable element, focus leaves the page.
        assert!(!e.key(&mut Mono, "Tab", false, false, false));
        assert_eq!(e.focus, None);
    }

    #[test]
    fn select_popup() {
        let mut b = Buf::default();
        b.create(1, "select")
            .listen(1, 3, "change:value")
            .insert(0, 1, 0);
        for (i, v) in [(2, "all"), (4, "open")] {
            b.create(i, "option")
                .attr(i, "value", v)
                .text(i + 10, v)
                .insert(i, i + 10, 0)
                .insert(1, i, 0);
        }
        b.int(dom::opc::SET_PROP).int(1).str("value").str("all");
        let mut e = engine(&b, "");
        click(&mut e, 1);
        let p = e.popup.clone().expect("popup open");
        let (x, y) = (p.rect.x + 10.0, p.rect.y + 1.0 + p.item_h * 1.5);
        e.pointer_move(&mut Mono, x, y);
        e.pointer_down(&mut Mono, x, y, false);
        e.pointer_up(&mut Mono, x, y);
        assert!(e.popup.is_none());
        assert_eq!(e.tree.n(1).value, "open");
        assert_eq!(emits(&mut e), vec![(3, Payload::Str("open".into()))]);
    }

    #[test]
    fn wheel_scrolls_inner_container_then_root() {
        let mut b = Buf::default();
        b.create(1, "div").class(1, "v-scroll s").insert(0, 1, 0);
        b.create(2, "div").class(2, "tall").insert(1, 2, 0);
        b.create(3, "div").class(3, "page").insert(0, 3, 0);
        let mut e = engine(
            &b,
            ".s{height:100px}.tall{height:300px}.page{height:2000px}",
        );
        e.wheel(10.0, 10.0, 0.0, 150.0);
        assert_eq!(e.tree.n(1).lay.scroll.1, 150.0);
        e.wheel(10.0, 10.0, 0.0, 150.0);
        assert_eq!(e.tree.n(1).lay.scroll.1, 200.0);
        e.frame(&mut Mono, 0.0);
        e.wheel(10.0, 10.0, 0.0, 100.0);
        assert_eq!(e.tree.n(0).lay.scroll.1, 100.0);
        assert_eq!(e.abs(3).y, 100.0 - 100.0);
    }

    #[test]
    fn stale_events_are_dropped() {
        let mut b = Buf::default();
        b.create(1, "button")
            .listen(1, 4, "click")
            .insert(0, 1, 0)
            .text(2, "x")
            .insert(1, 2, 0);
        let mut e = engine(&b, "");
        click(&mut e, 1);
        let mut rm = Buf::default();
        rm.int(dom::opc::REMOVE).int(1);
        e.apply(&rm.0).unwrap();
        assert!(e.poll().is_empty());
        assert_eq!(e.focus, None);
    }

    #[test]
    fn poll_delivers_one_event_per_batch() {
        let mut b = Buf::default();
        b.create(1, "div").listen(1, 1, "click").insert(0, 1, 0);
        b.create(2, "div")
            .listen(2, 2, "click")
            .insert(1, 2, 0)
            .text(3, "x")
            .insert(2, 3, 0);
        b.int(dom::opc::TITLE).str("T");
        let mut e = Engine::new("", 800.0, 600.0);
        e.apply(&b.0).unwrap();
        e.frame(&mut Mono, 0.0);
        click(&mut e, 2);
        let first = e.poll();
        assert_eq!(&first[..4], &out::TITLE.to_le_bytes());
        assert!(first.len() > 12);
        assert_eq!(
            &e.poll()[..8],
            &[out::EMIT.to_le_bytes(), 1i32.to_le_bytes()].concat()[..]
        );
        assert!(e.poll().is_empty());
    }

    #[test]
    fn dump_is_json() {
        let mut b = Buf::default();
        b.create(1, "p")
            .attr(1, "title", "a\"b")
            .insert(0, 1, 0)
            .text(2, "hi\n")
            .insert(1, 2, 0);
        let e = engine(&b, "");
        let d = e.dump();
        assert!(d.starts_with("[{\"id\":0"));
        assert!(d.contains("\"title\":\"a\\\"b\""));
        assert!(d.contains("\"text\":\"hi\\u000a\""));
    }
}

#[cfg(test)]
mod perf {
    use super::*;
    use crate::dom::tests::Buf;
    use crate::text::tests::Mono;

    /// Phase timings for a 1000-row list (run with `--ignored --nocapture`).
    #[test]
    #[ignore]
    fn phases() {
        let mut b = Buf::default();
        b.create(1, "div").class(1, "v-col").insert(0, 1, 0);
        b.create(2, "ul").class(2, "v-list s1").insert(1, 2, 0);
        for i in 0..1000 {
            let li = 10 + i * 3;
            b.create(li, "li")
                .class(li, "s2")
                .listen(li, i, "click")
                .insert(2, li, 0);
            b.text(li + 1, &format!("row {i}")).insert(li, li + 1, 0);
        }
        let css = ".s1{padding:0;list-style:none}.s2{padding:4px 8px;border-bottom:1px solid #ddd}.s3{color:#f00}.s4:hover{color:#00f}";
        let mut e = Engine::new(css, 1000.0, 800.0);
        e.apply(&b.0).unwrap();
        e.frame(&mut Mono, 0.0);
        let t = std::time::Instant::now();
        for _ in 0..20 {
            e.sheet.restyle(&mut e.tree, &e.media, true);
        }
        let style = t.elapsed() / 20;
        let sizes = HashMap::new();
        let mut m = Mono;
        let t = std::time::Instant::now();
        for _ in 0..20 {
            e.tree.mark_all();
            let mut cx = Cx {
                t: &mut e.tree,
                m: &mut m,
                fonts: &mut e.fonts,
                images: &sizes,
                vw: 1000.0,
                vh: 800.0,
            };
            layout::layout_root(&mut cx);
        }
        let lay = t.elapsed() / 20;
        // A swap of two rows: only the list is laid out again.
        let mut swap = Buf::default();
        swap.insert(2, 10 + 998 * 3, 10 + 3)
            .insert(2, 10 + 3, 10 + 999 * 3);
        let t = std::time::Instant::now();
        for _ in 0..20 {
            e.apply(&swap.0).unwrap();
            e.frame(&mut m, 0.0);
        }
        let swap_frame = t.elapsed() / 20;
        let t = std::time::Instant::now();
        for _ in 0..20 {
            let mut p = Painter::new(
                &e.tree,
                &mut e.fonts,
                &mut m,
                &e.images,
                &e.edit,
                None,
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1000.0,
                    h: 800.0,
                },
            );
            p.paint();
        }
        let paint = t.elapsed() / 20;
        println!("full: style {style:?} layout {lay:?} paint {paint:?}; swap frame {swap_frame:?}");
    }
}
