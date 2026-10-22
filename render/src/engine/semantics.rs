//! The accessibility mirror. The host keeps an invisible DOM element for each semantic element
//! and text run of the tree, placed over what the canvas shows, so that screen readers, search
//! engines and the browser's own text input (IME, autocorrect, virtual keyboards) work with
//! the application. Generic containers are left out; their children attach to the nearest
//! mirrored ancestor.

use super::{Engine, Payload, Writer};
use crate::dom::{Id, Kind};
use crate::style::Display;
use crate::text::Measure;

/// Elements mirrored under their own tag.
const TAGS: &[&str] = &[
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "p",
    "a",
    "button",
    "input",
    "textarea",
    "select",
    "option",
    "img",
    "nav",
    "main",
    "header",
    "footer",
    "section",
    "article",
    "aside",
    "ul",
    "ol",
    "li",
    "form",
    "strong",
    "em",
    "code",
    "pre",
    "blockquote",
    "hr",
    "table",
    "thead",
    "tbody",
    "tr",
    "th",
    "td",
];

/// Attributes copied besides `aria-*`.
const ATTRS: &[&str] = &[
    "href",
    "alt",
    "placeholder",
    "type",
    "role",
    "title",
    "lang",
    "value",
];

pub const FOCUSED: i32 = 1;
pub const CHECKED: i32 = 2;
pub const DISABLED: i32 = 4;

/// UTF-16 offset of byte offset `i` in `s` (browser selection offsets are UTF-16).
pub fn utf16_of(s: &str, i: usize) -> i32 {
    s[..i.min(s.len())].encode_utf16().count() as i32
}

/// Byte offset of UTF-16 offset `u` in `s`.
pub fn byte_of(s: &str, u: i32) -> usize {
    let mut n = 0;
    for (i, c) in s.char_indices() {
        if n >= u {
            return i;
        }
        n += c.len_utf16() as i32;
    }
    s.len()
}

fn entry(w: &mut Writer, id: Id, parent: i32, e: Writer) {
    // FNV-1a.
    let hash = e.0.iter().fold(0x811c9dc5u32, |h, &b| {
        (h ^ b as u32).wrapping_mul(0x01000193)
    });
    w.int(id as i32)
        .int(parent)
        .int(hash as i32)
        .int(e.0.len() as i32);
    w.0.extend(e.0);
}

impl Engine {
    /// The mirror as a flat pre-order list, or `None` if nothing changed since the last call.
    /// Each entry: id, mirrored parent (-1 for the top), a hash and the byte length of the rest,
    /// then tag (`#text` for text), viewport rectangle, flags, and attribute pairs. The host
    /// skips entries whose hash it has already seen for that id.
    pub fn semantics(&mut self, m: &mut dyn Measure) -> Option<Vec<u8>> {
        self.update_layout(m);
        if !self.sem_dirty {
            return None;
        }
        self.sem_dirty = false;
        let mut w = Writer(Vec::new());
        let mut stack: Vec<(Id, i32)> = vec![(0, -1)];
        while let Some((id, parent)) = stack.pop() {
            let n = self.tree.n(id);
            let mut kept = parent;
            // Text of form controls, and lone text runs, are carried in attributes.
            let mut carried = false;
            match n.kind {
                Kind::Comment => {}
                Kind::Text if n.text.trim().is_empty() => {}
                Kind::Text => {
                    let mut e = Writer(Vec::new());
                    e.str("#text").rect(self.text_rect(id)).int(0).int(2);
                    e.str("text").str(&n.text).str("font").str(&n.style.font);
                    entry(&mut w, id, parent, e);
                }
                Kind::Element => {
                    if n.style.display == Display::None {
                        continue;
                    }
                    if id != 0 && (TAGS.contains(&n.tag.as_str()) || n.attr("role").is_some()) {
                        kept = id as i32;
                        let mut attrs: Vec<(&str, &str)> = n
                            .attrs
                            .iter()
                            .filter(|(k, _)| k.starts_with("aria-") || ATTRS.contains(&k.as_str()))
                            .map(|(k, v)| (k.as_str(), v.as_str()))
                            .collect();
                        let label;
                        match n.tag.as_str() {
                            "input" | "textarea" | "select" => {
                                attrs.retain(|(k, _)| *k != "value");
                                attrs.push(("value", &n.value));
                            }
                            "option" => {
                                label = self.tree.text_of(id);
                                attrs.push(("label", &label));
                            }
                            _ => {}
                        }
                        // An element holding just one text run carries it, halving the
                        // number of mirror elements for the common case.
                        carried = matches!(n.tag.as_str(), "option" | "textarea" | "input");
                        if let Some(t) = self.only_text(id) {
                            carried = true;
                            let t = self.tree.n(t);
                            attrs.push(("text", &t.text));
                            attrs.push(("font", &t.style.font));
                        }
                        let flags = if self.focus == Some(id) { FOCUSED } else { 0 }
                            | if n.checked { CHECKED } else { 0 }
                            | if n.attr("disabled").is_some() {
                                DISABLED
                            } else {
                                0
                            };
                        let mut e = Writer(Vec::new());
                        e.str(&n.tag).rect(self.abs(id));
                        e.int(flags).int(attrs.len() as i32);
                        for (k, v) in attrs {
                            e.str(k).str(v);
                        }
                        entry(&mut w, id, parent, e);
                    }
                    if !carried {
                        stack.extend(n.children.iter().rev().map(|&c| (c, kept)));
                    }
                }
            }
        }
        Some(w.0)
    }

    /// The single non-blank text child of `id`, if it has no other content.
    fn only_text(&self, id: Id) -> Option<Id> {
        let mut found = None;
        for &c in &self.tree.n(id).children {
            let n = self.tree.n(c);
            match n.kind {
                Kind::Comment => {}
                Kind::Text if n.text.trim().is_empty() => {}
                Kind::Text if found.is_none() => found = Some(c),
                _ => return None,
            }
        }
        found
    }

    /// Focused element (or -1), its selection as UTF-16 offsets and its value (text fields).
    pub fn focus_state(&self) -> Vec<u8> {
        let mut w = Writer(Vec::new());
        let id = self.focus.map_or(-1, |f| f as i32);
        match self.edit.node {
            Some(f) if Some(f) == self.focus => {
                let v = &self.tree.n(f).value;
                w.int(id)
                    .int(utf16_of(v, self.edit.anchor))
                    .int(utf16_of(v, self.edit.caret))
                    .str(v)
            }
            _ => w.int(id).int(0).int(0).str(""),
        };
        w.0
    }

    /// Focus moved to a mirror element (browser Tab from outside, or a screen reader).
    pub fn focus_node(&mut self, id: Id) {
        if self.tree.get(id).is_some() && self.focusable(id) {
            self.set_focus(Some(id), true);
            self.scroll_into_view(id);
        }
    }

    /// A screen reader activated a mirror element.
    pub fn activate_node(&mut self, id: Id) {
        if self.tree.get(id).is_none()
            || self.closest(id, |n| n.attr("disabled").is_some()).is_some()
        {
            return;
        }
        self.focus_node(id);
        self.bubble(id, "click");
        self.activate(id);
    }

    /// The browser edited a mirrored text field: new value and selection (UTF-16 offsets).
    pub fn field_input(&mut self, id: Id, value: &str, start: i32, end: i32, backward: bool) {
        if self.tree.get(id).is_none() || !self.is_text_field(id) || self.disabled(id) {
            return;
        }
        if self.focus != Some(id) {
            self.set_focus(Some(id), true);
        }
        if self.tree.n(id).value != value {
            self.tree.m(id).value = value.to_string();
            self.emit(id, "input", Payload::Str(value.to_string()));
            self.invalidate();
        }
        let (a, z) = (byte_of(value, start), byte_of(value, end));
        let (anchor, caret) = if backward { (z, a) } else { (a, z) };
        if (anchor, caret) != (self.edit.anchor, self.edit.caret) {
            self.edit.anchor = anchor;
            self.edit.caret = caret;
            self.layout_dirty = true;
            self.restart_blink();
        }
    }

    /// A screen reader picked option `i` of a mirrored `select`.
    pub fn choose_index(&mut self, id: Id, i: usize) {
        if self.tree.get(id).is_some_and(|n| n.is("select")) && !self.disabled(id) {
            self.choose(id, i);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::tests::Buf;
    use crate::text::tests::Mono;

    /// (id, parent, tag, attributes) of each mirror entry.
    fn entries(bytes: &[u8]) -> Vec<(i32, i32, String, Vec<String>)> {
        let rd = |q: &mut usize| {
            let v = i32::from_le_bytes(bytes[*q..*q + 4].try_into().unwrap());
            *q += 4;
            v
        };
        let rs = |q: &mut usize| {
            let n = rd(q) as usize;
            let s = String::from_utf8(bytes[*q..*q + n].to_vec()).unwrap();
            *q += (n + 3) & !3;
            s
        };
        let (mut q, mut out) = (0, Vec::new());
        while q < bytes.len() {
            let (id, parent) = (rd(&mut q), rd(&mut q));
            q += 8; // hash and length
            let tag = rs(&mut q);
            q += 20; // rectangle and flags
            let attrs = (0..rd(&mut q))
                .map(|_| format!("{}={}", rs(&mut q), rs(&mut q)))
                .collect();
            out.push((id, parent, tag, attrs));
        }
        out
    }

    #[test]
    fn mirrors_semantic_elements_only() {
        let mut b = Buf::default();
        b.create(1, "div").insert(0, 1, 0);
        b.create(2, "h1")
            .insert(1, 2, 0)
            .text(3, "Hi")
            .insert(2, 3, 0);
        b.create(4, "input")
            .attr(4, "aria-label", "Name")
            .attr(4, "data-x", "1")
            .insert(1, 4, 0);
        let mut e = Engine::new("", 800.0, 600.0);
        e.apply(&b.0).unwrap();
        let list = entries(&e.semantics(&mut Mono).unwrap());
        let shape: Vec<_> = list
            .iter()
            .map(|(id, p, tag, _)| (*id, *p, tag.as_str()))
            .collect();
        assert_eq!(shape, vec![(2, -1, "h1"), (4, -1, "input")]);
        assert_eq!(list[0].3[0], "text=Hi");
        assert_eq!(list[1].3, vec!["aria-label=Name", "value="]);
        assert!(e.semantics(&mut Mono).is_none(), "unchanged");
        e.field_input(4, "zoé", 3, 3, false);
        assert_eq!(e.tree.n(4).value, "zoé");
        assert_eq!(e.edit.caret, "zoé".len());
        assert!(e.semantics(&mut Mono).is_some());
    }

    #[test]
    fn utf16_offsets() {
        let s = "a€😀b";
        assert_eq!(utf16_of(s, s.len()), 5);
        assert_eq!(byte_of(s, 4), "a€😀".len());
        assert_eq!(byte_of(s, 99), s.len());
    }
}
