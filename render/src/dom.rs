//! The element tree and the command buffer that builds it.
//!
//! Commands come from the application module in the format documented in
//! `src/lower_ui.rs` (`opc`): little-endian `i32` words, strings as a byte length followed by
//! UTF-8 bytes padded to a multiple of 4.

use crate::layout::Lay;
use crate::style::Style;

pub type Id = u32;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Element,
    Text,
    Comment,
}

pub const HOVER: u8 = 1;
pub const ACTIVE: u8 = 2;
pub const FOCUS: u8 = 4;
pub const FOCUS_VISIBLE: u8 = 8;
pub const FOCUS_WITHIN: u8 = 16;

pub struct Listener {
    pub event: String,
    pub payload: String,
    pub slot: i32,
}

pub struct Node {
    pub kind: Kind,
    pub tag: String,
    pub text: String,
    pub parent: Option<Id>,
    pub children: Vec<Id>,
    pub attrs: Vec<(String, String)>,
    pub classes: Vec<String>,
    pub listeners: Vec<Listener>,
    /// Current value of `input`, `textarea` and `select` elements.
    pub value: String,
    pub checked: bool,
    pub flags: u8,
    /// The computed style must be recomputed.
    pub style_dirty: bool,
    pub style: Style,
    pub lay: Lay,
}

impl Node {
    fn new(kind: Kind, tag: &str, text: String) -> Node {
        Node {
            kind,
            tag: tag.to_string(),
            text,
            parent: None,
            children: Vec::new(),
            attrs: Vec::new(),
            classes: Vec::new(),
            listeners: Vec::new(),
            value: String::new(),
            checked: false,
            flags: 0,
            style_dirty: true,
            style: Style::default(),
            lay: Lay::default(),
        }
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn is(&self, tag: &str) -> bool {
        self.kind == Kind::Element && self.tag == tag
    }

    pub fn has_class(&self, c: &str) -> bool {
        self.classes.iter().any(|x| x == c)
    }

    pub fn listener(&self, event: &str) -> Option<&Listener> {
        self.listeners.iter().find(|l| l.event == event)
    }
}

/// Side effects of commands that the browser host carries out.
#[derive(Debug, PartialEq)]
pub enum Effect {
    Title(String),
    Navigate(String),
    Warn(String),
    Fetch {
        slot: i32,
        method: i32,
        url: String,
        body: Option<String>,
    },
}

pub mod opc {
    pub const CREATE: i32 = 1;
    pub const TEXT: i32 = 2;
    pub const COMMENT: i32 = 3;
    pub const INSERT: i32 = 4;
    pub const REMOVE: i32 = 5;
    pub const SET_TEXT: i32 = 6;
    pub const SET_ATTR: i32 = 7;
    pub const REMOVE_ATTR: i32 = 8;
    pub const SET_PROP: i32 = 9;
    pub const SET_CHECKED: i32 = 10;
    pub const LISTEN: i32 = 11;
    pub const CLASS: i32 = 12;
    pub const TOGGLE_CLASS: i32 = 13;
    pub const TITLE: i32 = 14;
    pub const NAVIGATE: i32 = 15;
    pub const WARN: i32 = 16;
    pub const CLEAR: i32 = 17;
    pub const FETCH: i32 = 18;
}

pub struct Tree {
    pub nodes: Vec<Option<Node>>,
    /// Commands applied so far, and how many of them moved an attached node.
    pub commands: usize,
    pub moves: usize,
}

impl Default for Tree {
    fn default() -> Self {
        Tree {
            nodes: vec![Some(Node::new(Kind::Element, "body", String::new()))],
            commands: 0,
            moves: 0,
        }
    }
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn int(&mut self) -> Result<i32, String> {
        let w = self
            .b
            .get(self.p..self.p + 4)
            .ok_or("truncated command buffer")?;
        self.p += 4;
        Ok(i32::from_le_bytes(w.try_into().unwrap()))
    }

    fn str(&mut self) -> Result<String, String> {
        let n = self.int()? as usize;
        let s = self
            .b
            .get(self.p..self.p + n)
            .ok_or("truncated command buffer")?;
        self.p += (n + 3) & !3;
        Ok(String::from_utf8_lossy(s).into_owned())
    }
}

impl Tree {
    pub fn get(&self, id: Id) -> Option<&Node> {
        self.nodes.get(id as usize).and_then(|n| n.as_ref())
    }

    pub fn n(&self, id: Id) -> &Node {
        self.nodes[id as usize].as_ref().expect("live node")
    }

    pub fn m(&mut self, id: Id) -> &mut Node {
        self.nodes[id as usize].as_mut().expect("live node")
    }

    fn node(&mut self, id: i32) -> Result<&mut Node, String> {
        self.nodes
            .get_mut(id as usize)
            .and_then(|n| n.as_mut())
            .ok_or_else(|| format!("unknown node {id}"))
    }

    fn put(&mut self, id: i32, node: Node) -> Result<(), String> {
        let i = usize::try_from(id).map_err(|_| format!("invalid node id {id}"))?;
        if i == 0 {
            return Err("cannot replace the root node".into());
        }
        if self.nodes.len() <= i {
            self.nodes.resize_with(i + 1, || None);
        }
        self.nodes[i] = Some(node);
        Ok(())
    }

    pub fn mark_style(&mut self, id: Id) {
        if let Some(n) = self.nodes.get_mut(id as usize).and_then(|n| n.as_mut()) {
            n.style_dirty = true;
        }
    }

    /// Invalidates the layout of `id` and its ancestors.
    pub fn mark_layout(&mut self, id: Id) {
        let mut cur = Some(id);
        while let Some(c) = cur {
            let Some(n) = self.nodes.get_mut(c as usize).and_then(|n| n.as_mut()) else {
                break;
            };
            if !n.lay.invalidate() {
                break;
            }
            cur = n.parent;
        }
    }

    /// Marks every node for restyle and relayout.
    pub fn mark_all(&mut self) {
        for n in self.nodes.iter_mut().flatten() {
            n.style_dirty = true;
            n.lay.invalidate();
        }
    }

    fn detach(&mut self, id: Id) {
        if let Some(p) = self.n(id).parent {
            self.mark_layout(p);
            self.m(p).children.retain(|&c| c != id);
            self.m(id).parent = None;
        }
    }

    /// Deletes `id` and its descendants.
    fn drop_subtree(&mut self, id: Id) {
        let mut stack = vec![id];
        while let Some(i) = stack.pop() {
            if let Some(n) = self.nodes[i as usize].take() {
                stack.extend(n.children);
            }
        }
    }

    /// True if `a` is `b` or an ancestor of `b`.
    pub fn contains(&self, a: Id, b: Id) -> bool {
        let mut cur = Some(b);
        while let Some(c) = cur {
            if c == a {
                return true;
            }
            cur = self.get(c).and_then(|n| n.parent);
        }
        false
    }

    /// Applies a command buffer, appending host effects to `fx`.
    pub fn apply(&mut self, buf: &[u8], fx: &mut Vec<Effect>) -> Result<(), String> {
        let mut r = Reader { b: buf, p: 0 };
        while r.p < buf.len() {
            let op = r.int()?;
            self.commands += 1;
            match op {
                opc::CREATE => {
                    let id = r.int()?;
                    let tag = r.str()?;
                    self.put(id, Node::new(Kind::Element, &tag, String::new()))?;
                }
                opc::TEXT => {
                    let id = r.int()?;
                    let text = r.str()?;
                    self.put(id, Node::new(Kind::Text, "", text))?;
                }
                opc::COMMENT => {
                    let id = r.int()?;
                    self.put(id, Node::new(Kind::Comment, "", String::new()))?;
                }
                opc::INSERT => {
                    let (parent, node, anchor) = (r.int()?, r.int()?, r.int()?);
                    self.node(parent)?;
                    self.node(node)?;
                    let (parent, node) = (parent as Id, node as Id);
                    if self.contains(node, parent) {
                        return Err(format!("cannot insert node {node} into its own descendant"));
                    }
                    if self.n(node).parent.is_some() {
                        self.moves += 1;
                    }
                    self.detach(node);
                    let kids = &self.n(parent).children;
                    let at = if anchor == 0 {
                        kids.len()
                    } else {
                        kids.iter()
                            .position(|&c| c == anchor as Id)
                            .ok_or_else(|| format!("node {anchor} is not a child of {parent}"))?
                    };
                    self.m(parent).children.insert(at, node);
                    self.m(node).parent = Some(parent);
                    self.mark_layout(parent);
                    self.mark_style(node);
                }
                opc::REMOVE => {
                    let id = r.int()?;
                    self.node(id)?;
                    if id == 0 {
                        return Err("cannot remove the root node".into());
                    }
                    self.detach(id as Id);
                    self.drop_subtree(id as Id);
                }
                opc::SET_TEXT => {
                    let id = r.int()?;
                    let text = r.str()?;
                    self.node(id)?.text = text;
                    self.mark_layout(id as Id);
                }
                opc::SET_ATTR => {
                    let id = r.int()?;
                    let (name, value) = (r.str()?, r.str()?);
                    if name.len() > 2 && name[..2].eq_ignore_ascii_case("on") {
                        return Err(format!("refusing to set event-handler attribute {name}"));
                    }
                    let n = self.node(id)?;
                    match n.attrs.iter_mut().find(|(k, _)| *k == name) {
                        Some(slot) => slot.1 = value,
                        None => n.attrs.push((name, value)),
                    }
                    self.changed(id as Id);
                }
                opc::REMOVE_ATTR => {
                    let id = r.int()?;
                    let name = r.str()?;
                    self.node(id)?.attrs.retain(|(k, _)| *k != name);
                    self.changed(id as Id);
                }
                opc::SET_PROP => {
                    let id = r.int()?;
                    let (name, value) = (r.str()?, r.str()?);
                    if name != "value" {
                        return Err(format!("refusing to set property {name}"));
                    }
                    self.node(id)?.value = value;
                    self.mark_layout(id as Id);
                }
                opc::SET_CHECKED => {
                    let id = r.int()?;
                    let on = r.int()? != 0;
                    self.node(id)?.checked = on;
                    self.changed(id as Id);
                    // A checkbox label matches `:checked` through its input.
                    if let Some(p) = self.n(id as Id).parent {
                        self.mark_style(p);
                    }
                }
                opc::LISTEN => {
                    let (id, slot) = (r.int()?, r.int()?);
                    let spec = r.str()?;
                    let (event, payload) = spec.split_once(':').unwrap_or((&spec, ""));
                    let l = Listener {
                        event: event.to_string(),
                        payload: payload.to_string(),
                        slot,
                    };
                    self.node(id)?.listeners.push(l);
                }
                opc::CLASS => {
                    let id = r.int()?;
                    let s = r.str()?;
                    self.node(id)?.classes = s.split_whitespace().map(String::from).collect();
                    self.mark_style(id as Id);
                }
                opc::TOGGLE_CLASS => {
                    let id = r.int()?;
                    let on = r.int()? != 0;
                    let c = r.str()?;
                    let n = self.node(id)?;
                    n.classes.retain(|x| *x != c);
                    if on {
                        n.classes.push(c);
                    }
                    self.mark_style(id as Id);
                }
                opc::TITLE => fx.push(Effect::Title(r.str()?)),
                opc::NAVIGATE => fx.push(Effect::Navigate(r.str()?)),
                opc::WARN => fx.push(Effect::Warn(r.str()?)),
                opc::CLEAR => {
                    let id = r.int()?;
                    let kids = std::mem::take(&mut self.node(id)?.children);
                    for k in kids {
                        self.drop_subtree(k);
                    }
                    self.mark_layout(id as Id);
                }
                opc::FETCH => {
                    let (slot, method, has_body) = (r.int()?, r.int()?, r.int()?);
                    let (url, body) = (r.str()?, r.str()?);
                    fx.push(Effect::Fetch {
                        slot,
                        method,
                        url,
                        body: (has_body != 0).then_some(body),
                    });
                }
                _ => {
                    return Err(format!(
                        "corrupt command buffer (opcode {op} at {})",
                        r.p - 4
                    ));
                }
            }
        }
        Ok(())
    }

    /// Style and layout may depend on any attribute (`disabled`, `src`, `rows`, ...).
    fn changed(&mut self, id: Id) {
        self.mark_style(id);
        self.mark_layout(id);
    }

    /// Number of live nodes, excluding the root.
    pub fn live(&self) -> usize {
        self.nodes.iter().skip(1).filter(|n| n.is_some()).count()
    }

    /// Visible text of `id` and its descendants (option and comment nodes excluded).
    pub fn text_of(&self, id: Id) -> String {
        let mut s = String::new();
        self.collect_text(id, &mut s);
        s
    }

    fn collect_text(&self, id: Id, s: &mut String) {
        let n = self.n(id);
        match n.kind {
            Kind::Text => s.push_str(&n.text),
            Kind::Element => n.children.iter().for_each(|&c| self.collect_text(c, s)),
            Kind::Comment => {}
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Encodes commands like the application module does.
    #[derive(Default)]
    pub struct Buf(pub Vec<u8>);

    impl Buf {
        pub fn int(&mut self, v: i32) -> &mut Self {
            self.0.extend(v.to_le_bytes());
            self
        }
        pub fn str(&mut self, s: &str) -> &mut Self {
            self.int(s.len() as i32);
            self.0.extend(s.as_bytes());
            while !self.0.len().is_multiple_of(4) {
                self.0.push(0);
            }
            self
        }
        pub fn create(&mut self, id: i32, tag: &str) -> &mut Self {
            self.int(opc::CREATE).int(id).str(tag)
        }
        pub fn text(&mut self, id: i32, s: &str) -> &mut Self {
            self.int(opc::TEXT).int(id).str(s)
        }
        pub fn insert(&mut self, parent: i32, node: i32, anchor: i32) -> &mut Self {
            self.int(opc::INSERT).int(parent).int(node).int(anchor)
        }
        pub fn class(&mut self, id: i32, c: &str) -> &mut Self {
            self.int(opc::CLASS).int(id).str(c)
        }
        pub fn attr(&mut self, id: i32, k: &str, v: &str) -> &mut Self {
            self.int(opc::SET_ATTR).int(id).str(k).str(v)
        }
        pub fn listen(&mut self, id: i32, slot: i32, spec: &str) -> &mut Self {
            self.int(opc::LISTEN).int(id).int(slot).str(spec)
        }
    }

    #[test]
    fn builds_and_moves() {
        let mut t = Tree::default();
        let mut fx = Vec::new();
        let mut b = Buf::default();
        b.create(1, "ul").insert(0, 1, 0);
        for i in 2..5 {
            b.create(i, "li")
                .text(i + 10, &format!("item {i}"))
                .insert(i, i + 10, 0)
                .insert(1, i, 0);
        }
        t.apply(&b.0, &mut fx).unwrap();
        assert_eq!(t.n(1).children, vec![2, 3, 4]);
        let mut b = Buf::default();
        b.insert(1, 4, 2);
        t.apply(&b.0, &mut fx).unwrap();
        assert_eq!(t.n(1).children, vec![4, 2, 3]);
        assert_eq!(t.moves, 1);
        assert_eq!(t.text_of(1), "item 4item 2item 3");
        let mut b = Buf::default();
        b.int(opc::REMOVE).int(2);
        t.apply(&b.0, &mut fx).unwrap();
        assert_eq!(t.live(), 5);
        assert!(t.get(12).is_none());
    }

    #[test]
    fn rejects_bad_input() {
        let mut t = Tree::default();
        let mut fx = Vec::new();
        let mut b = Buf::default();
        b.create(1, "a").attr(1, "onclick", "x");
        assert!(
            t.apply(&b.0, &mut fx)
                .unwrap_err()
                .contains("event-handler")
        );
        assert!(
            t.apply(&[99, 0, 0, 0], &mut fx)
                .unwrap_err()
                .contains("opcode 99")
        );
        let mut b = Buf::default();
        b.create(2, "div")
            .create(3, "div")
            .insert(2, 3, 0)
            .insert(3, 2, 0);
        assert!(t.apply(&b.0, &mut fx).unwrap_err().contains("descendant"));
    }

    #[test]
    fn effects() {
        let mut t = Tree::default();
        let mut fx = Vec::new();
        let mut b = Buf::default();
        b.int(opc::TITLE)
            .str("Hi")
            .int(opc::FETCH)
            .int(7)
            .int(1)
            .int(1)
            .str("/api")
            .str("{}");
        t.apply(&b.0, &mut fx).unwrap();
        assert_eq!(
            fx,
            vec![
                Effect::Title("Hi".into()),
                Effect::Fetch {
                    slot: 7,
                    method: 1,
                    url: "/api".into(),
                    body: Some("{}".into())
                }
            ]
        );
    }
}
