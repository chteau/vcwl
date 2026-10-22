//! Checking of components and views: AST views -> typed views.

use crate::ast::{self, ElemArg, View};
use crate::check::{Checker, Ctx, Sym, suggest};
use crate::diag::{Diag, Result, Span};
use crate::style::Sheet;
use crate::tir::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VarKind {
    Prop,
    State,
    Derived,
}

/// Component-level names visible while checking a component.
pub struct UiEnv {
    pub vars: Vec<(String, Type, VarKind)>,
    /// Events this component can emit, with their payload types.
    pub events: Vec<(String, Option<Type>)>,
    /// Slots of this component and the type of the value each passes.
    pub slots: Vec<(String, Option<Type>)>,
    /// Locals bound by views (`for` items, view `match` bindings).
    pub bound: Vec<LocalId>,
}

impl UiEnv {
    pub fn names(&self) -> Vec<String> {
        self.vars.iter().map(|v| v.0.clone()).collect()
    }
}

#[derive(Clone, Debug)]
pub struct CompSig {
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub events: Vec<(String, Option<Type>)>,
    /// The body's slots for content given where the component is used: name (`""` for the
    /// unnamed slot) and the type of the value it passes to that content.
    pub slots: Vec<(String, Option<Type>)>,
    pub file: usize,
    pub item: usize,
}

#[derive(Clone, Debug)]
pub enum TVarKind {
    Prop(Option<TExpr>),
    State(TExpr),
    Derived(TExpr),
}

#[derive(Clone, Debug)]
pub struct TVar {
    pub name: String,
    pub ty: Type,
    pub kind: TVarKind,
}

/// How the host extracts an event payload, encoded in the `LISTEN` command.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Payload {
    None,
    Str,
    Bool,
    /// Component event raised with `emit`; the payload type is in `THandler::event_ty`.
    Event,
}

#[derive(Clone, Debug)]
pub struct THandler {
    pub param: Option<LocalId>,
    pub payload: Payload,
    pub event_ty: Option<Type>,
    pub body: TExpr,
}

#[derive(Clone, Debug)]
pub struct TComp {
    pub name: String,
    pub vars: Vec<TVar>,
    pub n_props: usize,
    pub locals: Vec<Local>,
    pub bound: Vec<LocalId>,
    pub body: Vec<TView>,
    pub handlers: Vec<THandler>,
    pub mount_handlers: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AttrKind {
    Attr(&'static str),
    Prop(&'static str),
    BoolAttr(&'static str),
    Checked,
    /// Internal route link: `href="#<path>"`.
    Route,
}

#[derive(Clone, Debug)]
pub enum Special {
    None,
    /// `<label><input type=checkbox>text</label>`; attributes and events go to the input.
    Checkbox(TExpr),
    /// `<select>` whose `<option>` children are rebuilt from a `[String]`.
    Select(TExpr),
}

#[derive(Clone, Debug)]
pub struct TElem {
    pub tag: String,
    pub static_attrs: Vec<(&'static str, String)>,
    pub attrs: Vec<(AttrKind, TExpr)>,
    pub classes: Vec<String>,
    pub class_ifs: Vec<(String, TExpr)>,
    /// Host event spec (e.g. `input:value`) and handler index.
    pub events: Vec<(String, usize)>,
    pub children: Vec<TView>,
    pub special: Special,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum SwitchKind {
    If(TExpr),
    Match(TExpr, Vec<TPat>),
    Routes(Vec<Option<String>>),
}

#[derive(Clone, Debug)]
pub enum TView {
    Element(TElem),
    Text(TExpr),
    Switch {
        kind: SwitchKind,
        branches: Vec<Vec<TView>>,
    },
    For {
        item: LocalId,
        index: Option<LocalId>,
        iter: TExpr,
        key: Option<TExpr>,
        body: Vec<TView>,
        span: Span,
    },
    /// Child component; `events` maps the child's event index to a handler of this component.
    /// `children` is content for the child's slots, rendered in this component's scope.
    Component {
        comp: usize,
        props: Vec<TExpr>,
        events: Vec<(usize, usize)>,
        children: Vec<TFill>,
        span: Span,
    },
    /// Where a component puts content given for the slot `name` (`""`: unnamed), passing it
    /// `value` for a slot declared with a type.
    Slot {
        name: String,
        value: Option<TExpr>,
    },
}

#[derive(Clone, Copy, PartialEq)]
enum ArgTy {
    Str,
    Bool,
    Int,
    Float,
    /// The prelude's `Vec3`, passed as `"x y z"`.
    Vec3,
}

struct Spec {
    tag: &'static str,
    base_class: &'static str,
    text_arg: bool,
    children: bool,
    layout: bool,
    args: &'static [(&'static str, ArgTy, AttrKind)],
    events: &'static [(&'static str, &'static str, Payload)],
    required: &'static [&'static str],
}

const COMMON: &[(&str, ArgTy, AttrKind)] = &[
    ("label", ArgTy::Str, AttrKind::Attr("aria-label")),
    ("title", ArgTy::Str, AttrKind::Attr("title")),
    ("id", ArgTy::Str, AttrKind::Attr("id")),
];
const CLICK: &[(&str, &str, Payload)] = &[
    ("click", "click", Payload::None),
    ("dblclick", "dblclick", Payload::None),
];
const FIELD_EVENTS: &[(&str, &str, Payload)] = &[
    ("input", "input:value", Payload::Str),
    ("change", "change:value", Payload::Str),
    ("keydown", "keydown:key", Payload::Str),
    ("focus", "focus", Payload::None),
    ("blur", "blur", Payload::None),
];

fn spec(name: &str) -> Option<Spec> {
    let container = |tag: &'static str, base_class: &'static str| Spec {
        tag,
        base_class,
        text_arg: false,
        children: true,
        layout: true,
        args: COMMON,
        events: CLICK,
        required: &[],
    };
    let texty = |tag: &'static str| Spec {
        tag,
        base_class: "",
        text_arg: true,
        children: true,
        layout: false,
        args: COMMON,
        events: CLICK,
        required: &[],
    };
    Some(match name {
        "column" => container("div", "v-col"),
        "row" => container("div", "v-row"),
        "stack" => container("div", "v-stack"),
        "grid" => container("div", "v-grid"),
        "box" => container("div", ""),
        "scroll" => container("div", "v-scroll"),
        "section" | "header" | "footer" | "nav" | "main" | "article" | "aside" => {
            let tag: &'static str = match name {
                "section" => "section",
                "header" => "header",
                "footer" => "footer",
                "nav" => "nav",
                "main" => "main",
                "article" => "article",
                _ => "aside",
            };
            container(tag, "")
        }
        "list" => Spec {
            layout: true,
            ..container("ul", "v-list")
        },
        "ordered_list" => Spec {
            layout: true,
            ..container("ol", "v-list")
        },
        "item" => texty("li"),
        "paragraph" => texty("p"),
        "span" => texty("span"),
        "strong" => texty("strong"),
        "code" => texty("code"),
        "heading" => texty("h"),
        "button" => Spec {
            args: &[
                ("disabled", ArgTy::Bool, AttrKind::BoolAttr("disabled")),
                ("pressed", ArgTy::Bool, AttrKind::Attr("aria-pressed")),
                ("label", ArgTy::Str, AttrKind::Attr("aria-label")),
                ("title", ArgTy::Str, AttrKind::Attr("title")),
            ],
            ..texty("button")
        },
        "link" => Spec {
            args: &[
                ("to", ArgTy::Str, AttrKind::Route),
                ("href", ArgTy::Str, AttrKind::Attr("href")),
                ("current", ArgTy::Bool, AttrKind::Attr("aria-current")),
                ("label", ArgTy::Str, AttrKind::Attr("aria-label")),
                ("title", ArgTy::Str, AttrKind::Attr("title")),
                ("id", ArgTy::Str, AttrKind::Attr("id")),
            ],
            ..texty("a")
        },
        // 3D: a `scene` is a box of the page drawn with WebGL; `mesh`, `light` and `group`
        // describe its content and exist only inside it.
        "scene" => Spec {
            tag: "scene",
            base_class: "",
            text_arg: false,
            children: true,
            layout: false,
            args: &[
                ("label", ArgTy::Str, AttrKind::Attr("aria-label")),
                ("camera", ArgTy::Vec3, AttrKind::Attr("camera")),
                ("target", ArgTy::Vec3, AttrKind::Attr("target")),
                ("fov", ArgTy::Float, AttrKind::Attr("fov")),
                ("background", ArgTy::Str, AttrKind::Attr("background")),
                ("orbit", ArgTy::Bool, AttrKind::Attr("orbit")),
                ("shadows", ArgTy::Bool, AttrKind::Attr("shadows")),
                ("id", ArgTy::Str, AttrKind::Attr("id")),
            ],
            events: &[],
            required: &["label"],
        },
        "mesh" => Spec {
            tag: "mesh",
            base_class: "",
            text_arg: false,
            children: false,
            layout: false,
            args: &[
                ("shape", ArgTy::Str, AttrKind::Attr("shape")),
                ("model", ArgTy::Str, AttrKind::Attr("model")),
                ("label", ArgTy::Str, AttrKind::Attr("aria-label")),
                ("size", ArgTy::Vec3, AttrKind::Attr("size")),
                ("position", ArgTy::Vec3, AttrKind::Attr("position")),
                ("rotation", ArgTy::Vec3, AttrKind::Attr("rotation")),
                ("spin", ArgTy::Vec3, AttrKind::Attr("spin")),
                ("color", ArgTy::Str, AttrKind::Attr("color")),
                ("shininess", ArgTy::Float, AttrKind::Attr("shininess")),
                ("wireframe", ArgTy::Bool, AttrKind::Attr("wireframe")),
                ("opacity", ArgTy::Float, AttrKind::Attr("opacity")),
                ("texture", ArgTy::Str, AttrKind::Attr("texture")),
                ("animation", ArgTy::Str, AttrKind::Attr("animation")),
            ],
            events: CLICK,
            required: &[],
        },
        "group" => Spec {
            tag: "group",
            base_class: "",
            text_arg: false,
            children: true,
            layout: false,
            args: &[
                ("position", ArgTy::Vec3, AttrKind::Attr("position")),
                ("rotation", ArgTy::Vec3, AttrKind::Attr("rotation")),
                ("scale", ArgTy::Vec3, AttrKind::Attr("size")),
                ("spin", ArgTy::Vec3, AttrKind::Attr("spin")),
            ],
            events: &[],
            required: &[],
        },
        "light" => Spec {
            tag: "light",
            base_class: "",
            text_arg: false,
            children: false,
            layout: false,
            args: &[
                ("kind", ArgTy::Str, AttrKind::Attr("kind")),
                ("position", ArgTy::Vec3, AttrKind::Attr("position")),
                ("color", ArgTy::Str, AttrKind::Attr("color")),
                ("intensity", ArgTy::Float, AttrKind::Attr("intensity")),
            ],
            events: &[],
            required: &[],
        },
        "image" => Spec {
            tag: "img",
            base_class: "",
            text_arg: false,
            children: false,
            layout: false,
            args: &[
                ("src", ArgTy::Str, AttrKind::Attr("src")),
                ("alt", ArgTy::Str, AttrKind::Attr("alt")),
                ("title", ArgTy::Str, AttrKind::Attr("title")),
            ],
            events: CLICK,
            required: &["src", "alt"],
        },
        "input" => Spec {
            tag: "input",
            base_class: "v-input",
            text_arg: false,
            children: false,
            layout: false,
            args: &[
                ("value", ArgTy::Str, AttrKind::Prop("value")),
                ("placeholder", ArgTy::Str, AttrKind::Attr("placeholder")),
                ("label", ArgTy::Str, AttrKind::Attr("aria-label")),
                ("disabled", ArgTy::Bool, AttrKind::BoolAttr("disabled")),
                ("title", ArgTy::Str, AttrKind::Attr("title")),
            ],
            events: FIELD_EVENTS,
            required: &["label"],
        },
        "textarea" => Spec {
            tag: "textarea",
            base_class: "v-input",
            text_arg: false,
            children: false,
            layout: false,
            args: &[
                ("value", ArgTy::Str, AttrKind::Prop("value")),
                ("placeholder", ArgTy::Str, AttrKind::Attr("placeholder")),
                ("label", ArgTy::Str, AttrKind::Attr("aria-label")),
                ("rows", ArgTy::Int, AttrKind::Attr("rows")),
                ("disabled", ArgTy::Bool, AttrKind::BoolAttr("disabled")),
            ],
            events: FIELD_EVENTS,
            required: &["label"],
        },
        "checkbox" => Spec {
            tag: "label",
            base_class: "v-check",
            text_arg: false,
            children: false,
            layout: false,
            args: &[
                ("checked", ArgTy::Bool, AttrKind::Checked),
                ("disabled", ArgTy::Bool, AttrKind::BoolAttr("disabled")),
            ],
            events: &[
                ("change", "change:checked", Payload::Bool),
                ("focus", "focus", Payload::None),
                ("blur", "blur", Payload::None),
            ],
            required: &["label"],
        },
        "select" => Spec {
            tag: "select",
            base_class: "v-input",
            text_arg: false,
            children: false,
            layout: false,
            args: &[
                ("value", ArgTy::Str, AttrKind::Prop("value")),
                ("label", ArgTy::Str, AttrKind::Attr("aria-label")),
                ("disabled", ArgTy::Bool, AttrKind::BoolAttr("disabled")),
            ],
            events: &[
                ("change", "change:value", Payload::Str),
                ("focus", "focus", Payload::None),
                ("blur", "blur", Payload::None),
            ],
            required: &["label", "options"],
        },
        "form" => Spec {
            events: &[("submit", "submit:prevent", Payload::None)],
            ..container("form", "v-col")
        },
        "divider" => Spec {
            tag: "hr",
            base_class: "",
            text_arg: false,
            children: false,
            layout: false,
            args: &[],
            events: &[],
            required: &[],
        },
        _ => return None,
    })
}

pub const ELEMENTS: &[&str] = &[
    "text",
    "column",
    "row",
    "stack",
    "grid",
    "box",
    "scroll",
    "section",
    "header",
    "footer",
    "nav",
    "main",
    "article",
    "aside",
    "list",
    "ordered_list",
    "item",
    "paragraph",
    "span",
    "strong",
    "code",
    "heading",
    "button",
    "link",
    "scene",
    "mesh",
    "group",
    "light",
    "image",
    "input",
    "textarea",
    "checkbox",
    "select",
    "form",
    "divider",
];

impl Checker<'_> {
    pub fn ui_ident(&mut self, name: &str, span: Span) -> Result<Option<TExpr>> {
        if name == "route" {
            return Ok(Some(TExpr {
                kind: TK::Route,
                ty: Type::Str,
                span,
            }));
        }
        let ui = self.ui.as_ref().unwrap();
        Ok(ui.vars.iter().position(|v| v.0 == name).map(|i| TExpr {
            kind: TK::CompVar(i),
            ty: ui.vars[i].1.clone(),
            span,
        }))
    }

    pub fn ui_place(&mut self, name: &str, span: Span) -> Result<(Place, Type)> {
        let ui = self.ui.as_ref().unwrap();
        let Some(i) = ui.vars.iter().position(|v| v.0 == name) else {
            return Err(Diag::ty(span, format!("unknown variable `{name}`")));
        };
        let (_, ty, kind) = ui.vars[i].clone();
        match kind {
            VarKind::State if self.ctx == Ctx::Handler => Ok((Place::CompVar(i), ty)),
            VarKind::State => Err(Diag::ty(span, format!("cannot modify state `{name}` while rendering"))
                .help("state can only change inside event handlers (`on click { ... }`)")),
            VarKind::Prop => Err(Diag::ty(span, format!("cannot assign to property `{name}`: properties are read-only"))
                .help("copy it into component state with `state local = prop` if it needs to change")),
            VarKind::Derived => Err(Diag::ty(span, format!("cannot assign to derived value `{name}`"))
                .help("derived values are recomputed from their inputs; assign to the state they depend on")),
        }
    }

    /// Resolves component property types (before any body is checked).
    pub fn declare_components(&mut self) -> Vec<CompSig> {
        let mut sigs = Vec::new();
        for (fi, f) in self.files.iter().enumerate() {
            for (ii, item) in f.items.iter().enumerate() {
                let ast::Item::Component(c) = item else {
                    continue;
                };
                self.file = fi;
                self.generics.clear();
                let mut params = Vec::new();
                for p in &c.params {
                    match self.resolve_type(&p.ty) {
                        Ok(t) => params.push((p.name.name.clone(), t)),
                        Err(d) => self.diags.push(d),
                    }
                }
                let mut events: Vec<(String, Option<Type>)> = Vec::new();
                for v in &c.body {
                    if let View::Event { name, ty, .. } = v {
                        if events.iter().any(|e| e.0 == name.name) {
                            self.diags.push(Diag::ty(
                                name.span,
                                format!("event `{}` is declared twice", name.name),
                            ));
                        }
                        let t = match ty {
                            Some(t) => match self.resolve_type(t) {
                                Ok(t) => Some(t),
                                Err(d) => {
                                    self.diags.push(d);
                                    None
                                }
                            },
                            None => None,
                        };
                        events.push((name.name.clone(), t));
                    }
                }
                if !c.name.name.starts_with(char::is_uppercase) {
                    self.diags.push(Diag::ty(
                        c.name.span,
                        format!(
                            "component names must start with an uppercase letter: `{}`",
                            c.name.name
                        ),
                    ));
                }
                let mut slots: Vec<(String, Option<Type>)> = Vec::new();
                for v in &c.body {
                    let View::SlotDecl { name, ty, .. } = v else {
                        continue;
                    };
                    if slots.iter().any(|s| s.0 == name.name) {
                        self.diags.push(Diag::ty(
                            name.span,
                            format!("slot `{}` is declared twice", name.name),
                        ));
                        continue;
                    }
                    match self.resolve_type(ty) {
                        Ok(t) => slots.push((name.name.clone(), Some(t))),
                        Err(d) => self.diags.push(d),
                    }
                }
                let mut sites = Vec::new();
                find_slots(&c.body, &mut sites);
                let mut seen: Vec<String> = Vec::new();
                for el in sites {
                    let (n, value) = match slot_args(el) {
                        Ok(x) => x,
                        Err(d) => {
                            self.diags.push(d);
                            continue;
                        }
                    };
                    if seen.contains(&n) {
                        self.diags.push(Diag::ty(
                            el.span,
                            if n.is_empty() {
                                "a component can have only one unnamed `slot`".to_string()
                            } else {
                                format!("slot `{n}` appears twice")
                            },
                        ));
                        continue;
                    }
                    seen.push(n.clone());
                    match (slots.iter().find(|s| s.0 == n), value) {
                        (Some((_, Some(_))), None) => self.diags.push(Diag::ty(
                            el.span,
                            format!("slot `{n}` passes a value: `slot({n}, value)`"),
                        )),
                        (None, Some(v)) => self.diags.push(
                            Diag::ty(v.span, "this slot is not declared with a value type")
                                .help(if n.is_empty() {
                                    "only named slots pass values: `slot item(Type)` and `slot(item, value)`".to_string()
                                } else {
                                    format!("declare it at the top of the component: `slot {n}(Type)`")
                                }),
                        ),
                        _ => {}
                    }
                    if !slots.iter().any(|s| s.0 == n) {
                        slots.push((n, None));
                    }
                }
                sigs.push(CompSig {
                    name: c.name.name.clone(),
                    params,
                    events,
                    slots,
                    file: fi,
                    item: ii,
                });
            }
        }
        sigs
    }

    pub fn check_component(
        &mut self,
        c: &ast::Component,
        sigs: &[CompSig],
        sheet: &mut Sheet,
    ) -> Result<(TComp, Option<String>)> {
        self.begin_body(Vec::new(), Type::Unit, Ctx::Render);
        let (events, slots) = sigs
            .iter()
            .find(|s| s.name == c.name.name && s.file == self.file)
            .map(|s| (s.events.clone(), s.slots.clone()))
            .unwrap_or_default();
        self.ui = Some(UiEnv {
            vars: Vec::new(),
            events,
            slots,
            bound: Vec::new(),
        });
        let r = self.check_component_inner(c, sigs, sheet);
        self.ui = None;
        r
    }

    fn check_component_inner(
        &mut self,
        c: &ast::Component,
        sigs: &[CompSig],
        sheet: &mut Sheet,
    ) -> Result<(TComp, Option<String>)> {
        let sig = sigs
            .iter()
            .find(|s| s.name == c.name.name && s.file == self.file)
            .unwrap()
            .clone();
        let mut vars = Vec::new();
        // Defaults are evaluated by the caller, so they cannot refer to other properties.
        for (p, (name, ty)) in c.params.iter().zip(&sig.params) {
            if vars.iter().any(|v: &TVar| v.name == *name) {
                return Err(Diag::ty(
                    p.name.span,
                    format!("duplicate property `{name}`"),
                ));
            }
            let default = match &p.default {
                Some(d) => {
                    let t = self.check_expr(d, Some(ty))?;
                    self.coerce(&t.ty, ty, d.span)?;
                    Some(t)
                }
                None => None,
            };
            vars.push(TVar {
                name: name.clone(),
                ty: ty.clone(),
                kind: TVarKind::Prop(default),
            });
        }
        for v in &vars {
            self.ui
                .as_mut()
                .unwrap()
                .vars
                .push((v.name.clone(), v.ty.clone(), VarKind::Prop));
        }
        let n_props = vars.len();
        // Pass 1: state and derived values, in order; each sees only earlier declarations.
        for v in &c.body {
            let (name, ty, value, is_state) = match v {
                View::State { name, ty, init, .. } => (name, ty, init, true),
                View::Let {
                    name, ty, value, ..
                } => (name, ty, value, false),
                _ => continue,
            };
            if self
                .ui
                .as_ref()
                .unwrap()
                .vars
                .iter()
                .any(|v| v.0 == name.name)
            {
                return Err(Diag::ty(
                    name.span,
                    format!("`{}` is already declared in this component", name.name),
                ));
            }
            let ann = match ty {
                Some(t) => Some(self.resolve_type(t)?),
                None => None,
            };
            let t = self.check_expr(value, ann.as_ref())?;
            if let Some(a) = &ann {
                self.coerce(&t.ty, a, value.span)?;
            }
            let ty = ann.unwrap_or_else(|| t.ty.clone());
            let ty = self.zonk(&ty);
            if ty == Type::Unit || ty == Type::Never {
                return Err(Diag::ty(
                    value.span,
                    "component values cannot have type `()`",
                ));
            }
            let mut t = t;
            self.zonk_expr(&mut t);
            let kind = if is_state {
                TVarKind::State(t)
            } else {
                TVarKind::Derived(t)
            };
            vars.push(TVar {
                name: name.name.clone(),
                ty: ty.clone(),
                kind,
            });
            let vk = if is_state {
                VarKind::State
            } else {
                VarKind::Derived
            };
            self.ui
                .as_mut()
                .unwrap()
                .vars
                .push((name.name.clone(), ty, vk));
        }
        // Pass 2: views and handlers.
        let mut cx = ViewCx {
            in_scene: false,
            sigs,
            sheet,
            handlers: Vec::new(),
            mount: Vec::new(),
            root_classes: Vec::new(),
        };
        let body = self.check_views(&c.body, &mut cx, true)?;
        let (handlers, mount, root_classes) = (cx.handlers, cx.mount, cx.root_classes);
        let mut body = body;
        if !root_classes.is_empty() {
            let elems: Vec<&mut TView> = body.iter_mut().collect();
            match elems.as_slice() {
                [TView::Element(_)] => {}
                _ => {
                    return Err(Diag::ty(
                        c.name.span,
                        "a component-level `style` needs exactly one root element",
                    )
                    .help(
                        "wrap the content in `column { ... }` or move the style into an element",
                    ));
                }
            }
            if let Some(TView::Element(e)) = body.first_mut() {
                e.classes.extend(root_classes);
            }
        }
        let bound = self.ui.as_ref().unwrap().bound.clone();
        let mut handlers = handlers;
        for h in &mut handlers {
            self.zonk_expr(&mut h.body);
        }
        for v in &mut body {
            self.zonk_view(v);
        }
        for i in 0..self.locals.len() {
            let z = self.zonk(&self.locals[i].ty);
            self.locals[i].ty = z;
        }
        Ok((
            TComp {
                name: c.name.name.clone(),
                vars,
                n_props,
                locals: std::mem::take(&mut self.locals),
                bound,
                body,
                handlers,
                mount_handlers: mount,
            },
            None,
        ))
    }

    fn zonk_view(&self, v: &mut TView) {
        match v {
            TView::Element(e) => {
                e.attrs.iter_mut().for_each(|(_, x)| self.zonk_expr(x));
                e.class_ifs.iter_mut().for_each(|(_, x)| self.zonk_expr(x));
                if let Special::Checkbox(x) | Special::Select(x) = &mut e.special {
                    self.zonk_expr(x);
                }
                e.children.iter_mut().for_each(|c| self.zonk_view(c));
            }
            TView::Text(x) => self.zonk_expr(x),
            TView::Switch { kind, branches } => {
                match kind {
                    SwitchKind::If(x) | SwitchKind::Match(x, _) => self.zonk_expr(x),
                    SwitchKind::Routes(_) => {}
                }
                branches
                    .iter_mut()
                    .flatten()
                    .for_each(|c| self.zonk_view(c));
            }
            TView::For {
                iter, key, body, ..
            } => {
                self.zonk_expr(iter);
                if let Some(k) = key {
                    self.zonk_expr(k);
                }
                body.iter_mut().for_each(|c| self.zonk_view(c));
            }
            TView::Component {
                props, children, ..
            } => {
                props.iter_mut().for_each(|x| self.zonk_expr(x));
                children
                    .iter_mut()
                    .flat_map(|f| &mut f.views)
                    .for_each(|c| self.zonk_view(c));
            }
            TView::Slot { value, .. } => {
                if let Some(v) = value {
                    self.zonk_expr(v);
                }
            }
        }
    }

    fn render_expr(&mut self, e: &ast::Expr, expected: Option<&Type>) -> Result<TExpr> {
        self.ctx = Ctx::Render;
        let t = self.check_expr(e, expected)?;
        if let Some(exp) = expected {
            self.coerce(&t.ty, exp, e.span)?;
        }
        Ok(t)
    }

    fn check_views(&mut self, views: &[View], cx: &mut ViewCx, top: bool) -> Result<Vec<TView>> {
        let mut out = Vec::new();
        for v in views {
            match v {
                View::Event { span, .. } => {
                    if !top {
                        return Err(Diag::ty(
                            *span,
                            "`event` must be declared at the top level of a component",
                        ));
                    }
                }
                View::SlotDecl { span, .. } => {
                    if !top {
                        return Err(Diag::ty(
                            *span,
                            "`slot name(Type)` must be declared at the top level of a component",
                        ));
                    }
                }
                View::State { span, .. } | View::Let { span, .. } => {
                    if !top {
                        return Err(Diag::ty(
                            *span,
                            "`state` and `let` must appear at the top level of a component",
                        ));
                    }
                }
                View::Element(el) => out.push(self.check_element(el, cx)?),
                View::If {
                    cond, then, els, ..
                } => {
                    let c = self.render_expr(cond, Some(&Type::Bool))?;
                    let t = self.scoped_views(then, cx)?;
                    let e = match els {
                        Some(e) => self.scoped_views(e, cx)?,
                        None => Vec::new(),
                    };
                    out.push(TView::Switch {
                        kind: SwitchKind::If(c),
                        branches: vec![t, e],
                    });
                }
                View::For {
                    var,
                    index,
                    iter,
                    key,
                    body,
                    span,
                } => {
                    let it = self.render_expr(iter, None)?;
                    let elem = match self.zonk(&it.ty) {
                        Type::Array(t) => *t,
                        t => {
                            return Err(Diag::ty(
                                iter.span,
                                format!("cannot iterate over `{}` in a view", self.show(&t)),
                            ));
                        }
                    };
                    self.local_scopes.push(Vec::new());
                    let r = (|| {
                        let item = self.bind(&var.name, elem, false);
                        self.ui.as_mut().unwrap().bound.push(item);
                        let idx = index.as_ref().map(|i| {
                            let l = self.bind(&i.name, Type::Int, false);
                            self.ui.as_mut().unwrap().bound.push(l);
                            l
                        });
                        let k = match key {
                            Some(k) => {
                                let tk = self.render_expr(k, None)?;
                                if !matches!(self.zonk(&tk.ty), Type::Int | Type::Str) {
                                    return Err(Diag::ty(
                                        k.span,
                                        format!(
                                            "list keys must be Int or String, found `{}`",
                                            self.show(&tk.ty)
                                        ),
                                    ));
                                }
                                Some(tk)
                            }
                            None => None,
                        };
                        let b = self.check_views(body, cx, false)?;
                        Ok(TView::For {
                            item,
                            index: idx,
                            iter: it,
                            key: k,
                            body: b,
                            span: *span,
                        })
                    })();
                    self.local_scopes.pop();
                    out.push(r?);
                }
                View::Match { scrut, arms, span } => {
                    let s = self.render_expr(scrut, None)?;
                    let sty = self.zonk(&s.ty);
                    let mut pats = Vec::new();
                    let mut branches = Vec::new();
                    for (p, body) in arms {
                        self.local_scopes.push(Vec::new());
                        let r = (|| {
                            let first = self.locals.len();
                            let tp = self.check_pat(p, &sty)?;
                            for l in first..self.locals.len() {
                                self.ui.as_mut().unwrap().bound.push(l);
                            }
                            Ok::<_, Diag>((tp, self.check_views(body, cx, false)?))
                        })();
                        self.local_scopes.pop();
                        let (tp, b) = r?;
                        pats.push(tp);
                        branches.push(b);
                    }
                    let fake: Vec<(TPat, TExpr)> = pats
                        .iter()
                        .map(|p| {
                            (
                                p.clone(),
                                TExpr {
                                    kind: TK::Unit,
                                    ty: Type::Unit,
                                    span: *span,
                                },
                            )
                        })
                        .collect();
                    self.check_exhaustive(&sty, &fake, *span)?;
                    out.push(TView::Switch {
                        kind: SwitchKind::Match(s, pats),
                        branches,
                    });
                }
                View::Routes { arms, span } => {
                    let mut paths = Vec::new();
                    let mut branches = Vec::new();
                    for (path, aspan, body) in arms {
                        if let Some(p) = path {
                            if !p.starts_with('/') {
                                return Err(Diag::ty(
                                    *aspan,
                                    format!("route paths must start with `/`: \"{p}\""),
                                ));
                            }
                            if paths.contains(&Some(p.clone())) {
                                return Err(Diag::ty(*aspan, format!("duplicate route \"{p}\"")));
                            }
                        }
                        paths.push(path.clone());
                        branches.push(self.scoped_views(body, cx)?);
                    }
                    if !paths.contains(&None) {
                        return Err(Diag::ty(
                            *span,
                            "`routes` needs a fallback arm `_ => ...` for unknown paths",
                        ));
                    }
                    if paths.last() != Some(&None) {
                        return Err(Diag::ty(
                            *span,
                            "the fallback route `_` must be the last arm",
                        ));
                    }
                    out.push(TView::Switch {
                        kind: SwitchKind::Routes(paths),
                        branches,
                    });
                }
                View::On {
                    event,
                    param,
                    body,
                    span,
                } => {
                    if event.name != "mount" {
                        return Err(Diag::ty(
                            *span,
                            format!(
                                "`on {}` must be placed inside the element it listens to",
                                event.name
                            ),
                        )
                        .help("only `on mount` is allowed at the top level of a component"));
                    }
                    if !top {
                        return Err(Diag::ty(
                            *span,
                            "`on mount` must appear at the top level of a component",
                        ));
                    }
                    if let Some(p) = param {
                        return Err(Diag::ty(p.span, "`mount` events carry no value"));
                    }
                    let h = self.check_handler(None, Payload::None, None, body)?;
                    cx.handlers.push(h);
                    cx.mount.push(cx.handlers.len() - 1);
                }
                View::Style(b) => {
                    if !top {
                        return Err(Diag::ty(
                            b.span,
                            "`style` must be inside an element or at the top level of a component",
                        ));
                    }
                    cx.root_classes.push(cx.sheet.block(b)?);
                }
                View::StyleRef(name) => {
                    if !top {
                        return Err(Diag::ty(
                            name.span,
                            "`style` must be inside an element or at the top level of a component",
                        ));
                    }
                    let c = self.named_style(name, cx)?;
                    cx.root_classes.push(c);
                }
                View::StyleIf { span, .. } | View::StyleBind { span, .. } => {
                    return Err(Diag::ty(
                        *span,
                        "this `style` must be placed inside an element",
                    ));
                }
            }
        }
        Ok(out)
    }

    fn scoped_views(&mut self, views: &[View], cx: &mut ViewCx) -> Result<Vec<TView>> {
        self.local_scopes.push(Vec::new());
        let r = self.check_views(views, cx, false);
        self.local_scopes.pop();
        r
    }

    fn named_style(&mut self, name: &ast::Ident, cx: &mut ViewCx) -> Result<String> {
        match self.scopes[self.file].get(&name.name) {
            Some(Sym::Style(i)) => Ok(cx.sheet.named[*i].clone()),
            _ => {
                let cands: Vec<String> = self.scopes[self.file]
                    .iter()
                    .filter(|(_, s)| matches!(s, Sym::Style(_)))
                    .map(|(n, _)| n.clone())
                    .collect();
                let mut d = Diag::ty(name.span, format!("unknown style `{}`", name.name));
                if let Some(s) = suggest(&name.name, cands.iter().map(|s| s.as_str())) {
                    d = d.help(format!("did you mean `{s}`?"));
                }
                Err(d)
            }
        }
    }

    fn check_handler(
        &mut self,
        param: Option<&ast::Ident>,
        payload: Payload,
        event_ty: Option<Type>,
        body: &ast::Block,
    ) -> Result<THandler> {
        self.ctx = Ctx::Handler;
        self.ret = Type::Unit;
        self.handler_base = self.local_scopes.len();
        self.local_scopes.push(Vec::new());
        let r = (|| {
            let p = match param {
                Some(p) => {
                    let t = match payload {
                        Payload::Bool => Type::Bool,
                        Payload::Event => event_ty.clone().unwrap(),
                        _ => Type::Str,
                    };
                    Some(self.bind(&p.name, t, false))
                }
                None => None,
            };
            let b = self.check_block(body, None)?;
            Ok(THandler {
                param: p,
                payload,
                event_ty: event_ty.clone(),
                body: b,
            })
        })();
        self.local_scopes.pop();
        self.ctx = Ctx::Render;
        r
    }

    fn check_element(&mut self, el: &ast::Element, cx: &mut ViewCx) -> Result<TView> {
        let name = el.name.name.as_str();
        if name.starts_with(char::is_uppercase) {
            return self.check_component_use(el, cx);
        }
        let three_d = matches!(name, "mesh" | "light" | "group");
        if three_d && !cx.in_scene {
            return Err(Diag::ty(
                el.name.span,
                format!("`{name}` can only be used inside a `scene`"),
            ));
        }
        if cx.in_scene && !three_d {
            return Err(
                Diag::ty(el.name.span, format!("`{name}` cannot be inside a `scene`"))
                    .help("a scene holds `mesh`, `light` and `group` elements"),
            );
        }
        if name == "slot" {
            let (n, value) = slot_args(el)?;
            let slots = &self.ui.as_ref().unwrap().slots;
            let idx = slots
                .iter()
                .position(|s| s.0 == n)
                .expect("slots collected");
            let value = match (slots[idx].1.clone(), value) {
                (Some(t), Some(v)) => Some(self.render_expr(v, Some(&t))?),
                _ => None,
            };
            let slot = TView::Slot { name: n, value };
            // A body is the fallback, shown when the user gives no content for the slot.
            if el.children.is_empty() {
                return Ok(slot);
            }
            let fallback = self.scoped_views(&el.children, cx)?;
            return Ok(TView::Switch {
                kind: SwitchKind::If(TExpr {
                    kind: TK::SlotFilled(idx),
                    ty: Type::Bool,
                    span: el.span,
                }),
                branches: vec![vec![slot], fallback],
            });
        }
        if name == "text" {
            if !el.children.is_empty() {
                return Err(Diag::ty(el.span, "`text` cannot have a body")
                    .help("use `span(...) { ... }` for styled or interactive text"));
            }
            let [ElemArg::Expr(a)] = el.args.as_slice() else {
                return Err(Diag::ty(
                    el.span,
                    "`text` takes exactly one value, e.g. `text(\"Hello\")`",
                ));
            };
            if a.name.is_some() {
                return Err(Diag::ty(el.span, "`text` takes one positional value"));
            }
            let t = self.render_expr(&a.value, None)?;
            return Ok(TView::Text(self.to_string_expr(t)?));
        }
        let Some(sp) = spec(name) else {
            let mut d = Diag::ty(el.name.span, format!("unknown element `{name}`"));
            if let Some(s) = suggest(name, ELEMENTS.iter().copied()) {
                d = d.help(format!("did you mean `{s}`?"));
            }
            return Err(d);
        };
        let mut te = TElem {
            tag: sp.tag.to_string(),
            static_attrs: Vec::new(),
            attrs: Vec::new(),
            classes: if sp.base_class.is_empty() {
                vec![]
            } else {
                vec![sp.base_class.to_string()]
            },
            class_ifs: Vec::new(),
            events: Vec::new(),
            children: Vec::new(),
            special: Special::None,
            span: el.span,
        };
        let mut layout_args = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        let mut text_child = None;
        let mut level = None;
        let mut checkbox_label = None;
        let mut options = None;
        let mut submit = false;
        for a in &el.args {
            match a {
                ElemArg::Style(id, toks, span) => {
                    if !sp.layout {
                        return Err(Diag::ty(
                            id.span,
                            format!(
                                "`{}` is a layout argument and is not supported on `{name}`",
                                id.name
                            ),
                        )
                        .help("put it in a `style { ... }` block inside the element"));
                    }
                    layout_args.push((id.clone(), toks.clone(), *span));
                }
                ElemArg::Expr(arg) => {
                    let Some(an) = &arg.name else {
                        if !sp.text_arg || text_child.is_some() {
                            return Err(Diag::ty(
                                arg.value.span,
                                format!("`{name}` does not take a positional value"),
                            )
                            .help("pass named arguments, e.g. `label: \"...\"`"));
                        }
                        let t = self.render_expr(&arg.value, None)?;
                        text_child = Some(self.to_string_expr(t)?);
                        continue;
                    };
                    if seen.contains(&an.name) {
                        return Err(Diag::ty(
                            an.span,
                            format!("argument `{}` is given twice", an.name),
                        ));
                    }
                    seen.push(an.name.clone());
                    match (name, an.name.as_str()) {
                        ("heading", "level") => {
                            let ast::ExprKind::Int(n @ 1..=6) = arg.value.kind else {
                                return Err(Diag::ty(
                                    arg.value.span,
                                    "`level` must be an integer literal from 1 to 6",
                                ));
                            };
                            level = Some(n);
                        }
                        ("input", "kind") => {
                            let kinds = [
                                "text", "email", "password", "number", "search", "url", "tel",
                            ];
                            let ok = match &arg.value.kind {
                                ast::ExprKind::Str(p) => match p.as_slice() {
                                    [ast::StrPart::Lit(s)] if kinds.contains(&s.as_str()) => {
                                        Some(s.clone())
                                    }
                                    _ => None,
                                },
                                _ => None,
                            };
                            let Some(k) = ok else {
                                return Err(Diag::ty(
                                    arg.value.span,
                                    format!("`kind` must be one of: {}", kinds.join(", ")),
                                ));
                            };
                            te.static_attrs.push(("type", k));
                        }
                        ("button", "submit") => {
                            let ast::ExprKind::Bool(b) = arg.value.kind else {
                                return Err(Diag::ty(
                                    arg.value.span,
                                    "`submit` must be `true` or `false`",
                                ));
                            };
                            submit = b;
                        }
                        ("checkbox", "label") => {
                            checkbox_label = Some(self.render_expr(&arg.value, Some(&Type::Str))?);
                        }
                        ("select", "options") => {
                            options = Some(self.render_expr(
                                &arg.value,
                                Some(&Type::Array(Box::new(Type::Str))),
                            )?);
                        }
                        (_, an_name) => {
                            let Some(&(_, ty, kind)) =
                                sp.args.iter().find(|(n, _, _)| *n == an_name)
                            else {
                                let mut d = Diag::ty(
                                    an.span,
                                    format!("`{name}` has no argument `{an_name}`"),
                                );
                                let cands: Vec<&str> = sp.args.iter().map(|a| a.0).collect();
                                if let Some(s) = suggest(an_name, cands.into_iter()) {
                                    d = d.help(format!("did you mean `{s}`?"));
                                }
                                return Err(d);
                            };
                            let want = match ty {
                                ArgTy::Str => Type::Str,
                                ArgTy::Bool => Type::Bool,
                                ArgTy::Int => Type::Int,
                                ArgTy::Float => Type::Float,
                                ArgTy::Vec3 => Type::Adt(self.prelude_adt("Vec3"), vec![]),
                            };
                            let mut t = self.render_expr(&arg.value, Some(&want))?;
                            self.coerce(&t.ty, &want, arg.value.span)?;
                            if ty == ArgTy::Vec3 {
                                let f = self.prelude_fn("vec3_text").unwrap();
                                let span = t.span;
                                t = TExpr {
                                    kind: TK::Call(f, vec![], vec![t]),
                                    ty: Type::Str,
                                    span,
                                };
                            } else if matches!(ty, ArgTy::Int | ArgTy::Float)
                                || ty == ArgTy::Bool && matches!(kind, AttrKind::Attr(_))
                            {
                                t = self.to_string_expr(t)?;
                            }
                            te.attrs.push((kind, t));
                        }
                    }
                }
            }
        }
        if name == "mesh"
            && !seen.iter().any(|s| s == "label")
            && el.children.iter().any(|c| matches!(c, View::On { .. }))
        {
            return Err(Diag::ty(
                el.name.span,
                "a clickable `mesh` requires a `label` argument (keyboard and screen reader users reach it as a button)",
            ));
        }
        for r in sp.required {
            if !seen.iter().any(|s| s == r) {
                let why = match *r {
                    "alt" => {
                        " (describe the image for screen readers; use \"\" only for decorative images)"
                    }
                    "label" => " (an accessible name is required)",
                    _ => "",
                };
                return Err(Diag::ty(
                    el.name.span,
                    format!(
                        "`{name}` requires {} `{r}` argument{why}",
                        if r.starts_with(['a', 'e', 'i', 'o', 'u']) {
                            "an"
                        } else {
                            "a"
                        }
                    ),
                ));
            }
        }
        if name == "heading" {
            te.tag = format!("h{}", level.unwrap_or(2));
        }
        if name == "button" {
            te.static_attrs
                .push(("type", if submit { "submit" } else { "button" }.into()));
        }
        if name == "scene" {
            te.static_attrs.push(("role", "img".into()));
        }
        if name == "link" && te.attrs.iter().any(|(k, _)| *k == AttrKind::Attr("href")) {
            te.static_attrs.push(("rel", "noopener noreferrer".into()));
        }
        if let Some(l) = checkbox_label {
            te.special = Special::Checkbox(l);
        }
        if let Some(o) = options {
            te.special = Special::Select(o);
        }
        if !layout_args.is_empty() {
            te.classes.push(cx.sheet.layout_args(name, &layout_args)?);
        }
        if let Some(t) = text_child {
            te.children.push(TView::Text(t));
        }
        // Body: children, events and styles.
        let mut children_views = Vec::new();
        for c in &el.children {
            match c {
                View::On {
                    event,
                    param,
                    body,
                    span,
                } => {
                    let Some(&(_, host, payload)) =
                        sp.events.iter().find(|(n, _, _)| *n == event.name)
                    else {
                        let names: Vec<&str> = sp.events.iter().map(|e| e.0).collect();
                        let mut d = Diag::ty(
                            event.span,
                            format!("`{name}` has no `{}` event", event.name),
                        );
                        d = if names.is_empty() {
                            d.help(format!("`{name}` does not emit events"))
                        } else {
                            d.help(format!("available events: {}", names.join(", ")))
                        };
                        return Err(d);
                    };
                    if let Some(p) = param
                        && payload == Payload::None
                    {
                        return Err(Diag::ty(
                            p.span,
                            format!("`{}` events carry no value", event.name),
                        )
                        .help(format!("write `on {} {{ ... }}`", event.name)));
                    }
                    let _ = span;
                    let h = self.check_handler(param.as_ref(), payload, None, body)?;
                    cx.handlers.push(h);
                    te.events.push((host.to_string(), cx.handlers.len() - 1));
                }
                View::Style(b) => te.classes.push(cx.sheet.block(b)?),
                View::StyleRef(n) => {
                    let c = self.named_style(n, cx)?;
                    te.classes.push(c);
                }
                View::StyleIf { cond, block, .. } => {
                    let t = self.render_expr(cond, Some(&Type::Bool))?;
                    te.class_ifs.push((cx.sheet.block(block)?, t));
                }
                View::StyleBind {
                    prop, value, unit, ..
                } => {
                    let Some((css, needs_unit)) = crate::style::bind_prop(&prop.name) else {
                        return Err(Diag::ty(
                            prop.span,
                            format!("`{}` cannot be bound to a value", prop.name),
                        )
                        .help("bind takes a property with one length (`width`, `left`, `size`, ...) or one number (`opacity`, `grow`)"));
                    };
                    if needs_unit != unit.is_some() {
                        return Err(Diag::ty(
                            value.span,
                            if needs_unit {
                                format!(
                                    "`{}` needs a unit after the value, e.g. `%` or `px`",
                                    prop.name
                                )
                            } else {
                                format!("`{}` takes a plain number", prop.name)
                            },
                        ));
                    }
                    let t = self.render_expr(value, None)?;
                    if !matches!(t.ty, Type::Int | Type::Float) {
                        return Err(Diag::ty(
                            value.span,
                            format!(
                                "a bound style value must be an Int or Float, found `{}`",
                                self.show(&t.ty)
                            ),
                        ));
                    }
                    let t = self.to_string_expr(t)?;
                    // The renderer reads `style:<property>:<unit>` attributes after the classes.
                    let name = format!("style:{css}:{}", unit.as_deref().unwrap_or(""));
                    te.attrs
                        .push((AttrKind::Attr(Box::leak(name.into_boxed_str())), t));
                }
                other => {
                    if !sp.children {
                        return Err(Diag::ty(
                            el.span,
                            format!("`{name}` cannot contain other elements"),
                        ));
                    }
                    children_views.push(other.clone());
                }
            }
        }
        let outer = cx.in_scene;
        cx.in_scene |= name == "scene";
        let kids = self.scoped_views(&children_views, cx);
        cx.in_scene = outer;
        let kids = kids?;
        te.children.extend(kids);
        Ok(TView::Element(te))
    }

    fn check_component_use(&mut self, el: &ast::Element, cx: &mut ViewCx) -> Result<TView> {
        let name = &el.name.name;
        let idx = match self.scopes[self.file].get(name) {
            Some(Sym::Comp(i)) => *i,
            _ => {
                let cands: Vec<String> = self.scopes[self.file]
                    .iter()
                    .filter(|(_, s)| matches!(s, Sym::Comp(_)))
                    .map(|(n, _)| n.clone())
                    .collect();
                let mut d = Diag::ty(el.name.span, format!("unknown component `{name}`"));
                if let Some(s) = suggest(name, cands.iter().map(|s| s.as_str())) {
                    d = d.help(format!("did you mean `{s}`?"));
                }
                return Err(d);
            }
        };
        let sig = cx.sigs[idx].clone();
        let mut events = Vec::new();
        // Content: `slot(name) { ... }` blocks fill named slots, the rest the unnamed one;
        // `slot(name, x) { ... }` names the value the slot passes.
        let mut content: Vec<(String, Option<ast::Ident>, Span, Vec<View>)> = Vec::new();
        for c in &el.children {
            match c {
                View::On { .. } => {}
                View::Element(f) if f.name.name == "slot" && !f.args.is_empty() => {
                    let (n, bind) = slot_args(f)?;
                    let bind = match bind.map(|b| &b.kind) {
                        None => None,
                        Some(ast::ExprKind::Ident(x)) => Some(ast::Ident {
                            name: x.clone(),
                            span: bind.unwrap().span,
                        }),
                        Some(_) => {
                            return Err(Diag::ty(
                                bind.unwrap().span,
                                "name the slot's value with a plain name, e.g. `slot(row, r) { ... }`",
                            ));
                        }
                    };
                    if content.iter().any(|(m, ..)| *m == n) {
                        return Err(Diag::ty(f.span, format!("slot `{n}` is filled twice")));
                    }
                    content.push((n, bind, f.span, f.children.clone()));
                }
                other => match content.iter_mut().find(|(m, ..)| m.is_empty()) {
                    Some(d) => d.3.push(other.clone()),
                    None => content.push((String::new(), None, el.span, vec![other.clone()])),
                },
            }
        }
        for (n, bind, span, _) in &content {
            if let Some((_, t)) = sig.slots.iter().find(|s| s.0 == *n) {
                if let Some(b) = bind
                    && t.is_none()
                {
                    return Err(Diag::ty(
                        b.span,
                        format!("slot `{n}` of `{name}` passes no value"),
                    ));
                }
                continue;
            }
            return Err(if n.is_empty() {
                Diag::ty(
                    *span,
                    format!("component `{name}` cannot take child content"),
                )
                .help(format!(
                    "add `slot` where the content goes in `{name}`, or pass data through properties"
                ))
            } else {
                let mut d = Diag::ty(*span, format!("component `{name}` has no slot `{n}`"));
                if let Some(s) = suggest(n, sig.slots.iter().map(|s| s.0.as_str())) {
                    d = d.help(format!("did you mean `{s}`?"));
                }
                d
            });
        }
        for c in &el.children {
            let View::On {
                event, param, body, ..
            } = c
            else {
                continue;
            };
            let Some(ei) = sig.events.iter().position(|e| e.0 == event.name) else {
                let mut d = Diag::ty(
                    event.span,
                    format!("component `{name}` has no event `{}`", event.name),
                );
                if let Some(s) = suggest(&event.name, sig.events.iter().map(|e| e.0.as_str())) {
                    d = d.help(format!("did you mean `{s}`?"));
                } else if sig.events.is_empty() {
                    d = d.help(format!(
                        "declare it in `{name}` with `event {}(Type)`",
                        event.name
                    ));
                }
                return Err(d);
            };
            let ety = sig.events[ei].1.clone();
            if let Some(p) = param
                && ety.is_none()
            {
                return Err(Diag::ty(
                    p.span,
                    format!("event `{}` carries no value", event.name),
                ));
            }
            if events.iter().any(|(e, _)| *e == ei) {
                return Err(Diag::ty(
                    event.span,
                    format!("event `{}` is handled twice", event.name),
                ));
            }
            let h = self.check_handler(param.as_ref(), Payload::Event, ety, body)?;
            cx.handlers.push(h);
            events.push((ei, cx.handlers.len() - 1));
        }
        let ast::Item::Component(decl) = &self.files[sig.file].items[sig.item] else {
            unreachable!()
        };
        let mut given: Vec<Option<TExpr>> = vec![None; sig.params.len()];
        for a in &el.args {
            let (an, value) = match a {
                ElemArg::Expr(ast::Arg {
                    name: Some(n),
                    value,
                }) => (n.clone(), Some(value)),
                ElemArg::Style(n, _, _) => (n.clone(), None),
                ElemArg::Expr(a) => {
                    return Err(Diag::ty(
                        a.value.span,
                        "component properties are passed by name, e.g. `Card(title: \"Hi\")`",
                    ));
                }
            };
            let Some(i) = sig.params.iter().position(|(n, _)| *n == an.name) else {
                let mut d = Diag::ty(
                    an.span,
                    format!("component `{name}` has no property `{}`", an.name),
                );
                if let Some(s) = suggest(&an.name, sig.params.iter().map(|p| p.0.as_str())) {
                    d = d.help(format!("did you mean `{s}`?"));
                }
                return Err(d);
            };
            let Some(value) = value else {
                return Err(Diag::ty(
                    an.span,
                    format!("`{}` must be an expression here", an.name),
                ));
            };
            if given[i].is_some() {
                return Err(Diag::ty(
                    an.span,
                    format!("property `{}` is given twice", an.name),
                ));
            }
            given[i] = Some(self.render_expr(value, Some(&sig.params[i].1))?);
        }
        let mut props = Vec::new();
        for (i, g) in given.into_iter().enumerate() {
            match g {
                Some(t) => props.push(t),
                None if decl.params[i].default.is_some() => {
                    // The callee evaluates its own default.
                    props.push(TExpr {
                        kind: TK::Unit,
                        ty: Type::Never,
                        span: el.span,
                    });
                }
                None => {
                    return Err(Diag::ty(
                        el.name.span,
                        format!("component `{name}` requires property `{}`", sig.params[i].0),
                    ));
                }
            }
        }
        let mut children = Vec::new();
        for (n, bind, _, views) in &content {
            self.local_scopes.push(Vec::new());
            let r = (|| {
                let bind = bind.as_ref().map(|b| {
                    let t = sig
                        .slots
                        .iter()
                        .find(|s| s.0 == *n)
                        .unwrap()
                        .1
                        .clone()
                        .unwrap();
                    let l = self.bind(&b.name, t, false);
                    self.ui.as_mut().unwrap().bound.push(l);
                    l
                });
                Ok(TFill {
                    name: n.clone(),
                    bind,
                    views: self.check_views(views, cx, false)?,
                })
            })();
            self.local_scopes.pop();
            children.push(r?);
        }
        Ok(TView::Component {
            comp: idx,
            props,
            events,
            children,
            span: el.span,
        })
    }

    pub fn check_emit(
        &mut self,
        name: &ast::Ident,
        value: Option<&ast::Expr>,
        span: Span,
    ) -> Result<TStmt> {
        if self.ctx != Ctx::Handler || self.ui.is_none() {
            return Err(Diag::ty(
                span,
                "`emit` can only be used in a component's event handler",
            ));
        }
        let events = self.ui.as_ref().unwrap().events.clone();
        let Some(i) = events.iter().position(|e| e.0 == name.name) else {
            let mut d = Diag::ty(
                name.span,
                format!("this component has no event `{}`", name.name),
            );
            d = match suggest(&name.name, events.iter().map(|e| e.0.as_str())) {
                Some(s) => d.help(format!("did you mean `{s}`?")),
                None => d.help(format!(
                    "declare it at the top of the component: `event {}(Type)`",
                    name.name
                )),
            };
            return Err(d);
        };
        match (&events[i].1, value) {
            (None, None) => Ok(TStmt::Emit(i, None)),
            (Some(t), Some(v)) => {
                let tv = self.check_expr(v, Some(t))?;
                self.coerce(&tv.ty, t, v.span)?;
                Ok(TStmt::Emit(i, Some(tv)))
            }
            (None, Some(v)) => Err(Diag::ty(
                v.span,
                format!("event `{}` carries no value", name.name),
            )),
            (Some(t), None) => Err(Diag::ty(
                span,
                format!(
                    "event `{}` needs a value of type `{}`",
                    name.name,
                    self.show(t)
                ),
            )),
        }
    }

    /// `fetch method(url[, body]) as T { arms }`. The arms run when the response arrives,
    /// after the handler has returned, so they see component values but not handler locals.
    pub fn check_fetch(&mut self, f: &ast::Fetch) -> Result<TStmt> {
        if self.ctx != Ctx::Handler || self.ui.is_none() {
            return Err(Diag::ty(
                f.span,
                "`fetch` can only be used in a component's event handler",
            ));
        }
        const METHODS: [&str; 5] = ["get", "post", "put", "patch", "delete"];
        // Server functions, and JavaScript functions (`extern fn`) that may return a Promise.
        let server_fn = match self.scopes[self.file].get(&f.method.name) {
            Some(Sym::Fn(id)) if self.sigs[*id].server || self.sigs[*id].ext => Some(*id),
            _ => None,
        };
        let (method, url, body, ty) = if let Some(id) = server_fn {
            // `fetch save_todo(t) { ... }`: a POST of the JSON arguments to the server.
            let sig = self.sigs[id].clone();
            // A JavaScript function's result can be decoded as another type, checked at run
            // time like a response: `fetch select("todos", "") as [Todo]`.
            let ret = match &f.ty {
                Some(t) if sig.ext => self.resolve_type(t)?,
                Some(t) => {
                    return Err(Diag::ty(
                        t.span,
                        "a server function's result type is known: remove `as ...`",
                    ));
                }
                None => sig.ret.clone(),
            };
            if f.args.len() != sig.params.len() {
                return Err(Diag::ty(
                    f.span,
                    format!(
                        "`{}` takes {} argument(s) but {} were given",
                        sig.name,
                        sig.params.len(),
                        f.args.len()
                    ),
                ));
            }
            let mut args = Vec::new();
            for (a, p) in f.args.iter().zip(&sig.params) {
                let v = self.check_expr(a, Some(p))?;
                self.coerce(&v.ty, p, a.span)?;
                args.push(v);
            }
            let url = TExpr {
                kind: TK::Str(if sig.ext {
                    format!("js:{}", sig.name)
                } else {
                    format!("__vcwl/fn/{}", sig.name)
                }),
                ty: Type::Str,
                span: f.method.span,
            };
            let body = self.server_args(args, f.span)?;
            ("POST", url, Some(body), ret)
        } else {
            let Some(m) = METHODS.iter().position(|m| *m == f.method.name) else {
                let d = Diag::ty(
                    f.method.span,
                    format!("unknown HTTP method or server function `{}`", f.method.name),
                );
                return Err(match suggest(&f.method.name, METHODS.iter().copied()) {
                    Some(s) => d.help(format!("did you mean `{s}`?")),
                    None => d.help("use get, post, put, patch or delete, or a `server fn`"),
                });
            };
            let method = ["GET", "POST", "PUT", "PATCH", "DELETE"][m];
            let (Some(url_expr), true) = (f.args.first(), f.args.len() <= 2) else {
                return Err(Diag::ty(f.span, "`fetch` takes a URL and an optional body"));
            };
            let url = self.check_expr(url_expr, Some(&Type::Str))?;
            self.coerce(&url.ty, &Type::Str, url_expr.span)?;
            let body = match f.args.get(1) {
                Some(b) if matches!(method, "GET" | "DELETE") => {
                    return Err(Diag::ty(
                        b.span,
                        format!("`{}` requests cannot have a body", f.method.name),
                    ));
                }
                Some(b) => {
                    let v = self.check_expr(b, None)?;
                    let ty = self.zonk(&v.ty);
                    let enc = self.json_encoder(&ty, b.span)?;
                    Some(TExpr {
                        kind: TK::Call(enc, vec![], vec![v]),
                        ty: Type::Str,
                        span: b.span,
                    })
                }
                None => None,
            };
            let Some(t) = &f.ty else {
                return Err(Diag::ty(f.span, "`fetch` needs the expected response type")
                    .help("name it: `fetch get(url) as [User] { ... }`"));
            };
            (method, url, body, self.resolve_type(t)?)
        };
        let result_fn = self.fetch_result_fn(&ty, f.span)?;
        if server_fn.is_none() {
            self.fetch_span.get_or_insert(f.span);
        }

        self.fetch_hidden = self.local_scopes.split_off(self.handler_base);
        self.local_scopes.push(Vec::new());
        let r = (|| {
            let params = ["$kind", "$status", "$len"].map(|n| self.bind(n, Type::Int, false));
            let http = Type::Adt(self.prelude_adt("HttpError"), vec![]);
            let rty = Type::Adt(self.prelude_adt("Result"), vec![ty.clone(), http]);
            let result = self.bind("$result", rty.clone(), false);
            let args = params
                .iter()
                .map(|&p| TExpr {
                    kind: TK::Local(p),
                    ty: Type::Int,
                    span: f.span,
                })
                .collect();
            let call = TExpr {
                kind: TK::Call(result_fn, vec![], args),
                ty: rty,
                span: f.span,
            };
            let scrut = ast::Expr {
                id: 0,
                kind: ast::ExprKind::Ident("$result".into()),
                span: f.span,
            };
            let m = ast::Expr {
                id: 0,
                kind: ast::ExprKind::Match {
                    scrut: Box::new(scrut),
                    arms: f.arms.clone(),
                },
                span: f.span,
            };
            let arms = self.check_expr(&m, Some(&Type::Unit))?;
            let on_done = TExpr {
                kind: TK::Block(vec![TStmt::Let(result, call), TStmt::Expr(arms)], None),
                ty: Type::Unit,
                span: f.span,
            };
            Ok(TStmt::Fetch(Box::new(TFetch {
                method,
                url,
                body,
                params,
                on_done,
                span: f.span,
            })))
        })();
        self.local_scopes.pop();
        let hidden = std::mem::take(&mut self.fetch_hidden);
        self.local_scopes.extend(hidden);
        r
    }
}

struct ViewCx<'s> {
    /// Checking the content of a `scene`.
    in_scene: bool,
    sigs: &'s [CompSig],
    sheet: &'s mut Sheet,
    handlers: Vec<THandler>,
    mount: Vec<usize>,
    root_classes: Vec<String>,
}

/// `slot` elements of a component body, including inside the content it gives other
/// components (forwarding), but not the `slot(name) { ... }` blocks that fill their slots.
fn find_slots<'a>(views: &'a [View], out: &mut Vec<&'a ast::Element>) {
    for v in views {
        match v {
            View::Element(el) if el.name.name == "slot" => {
                out.push(el);
                find_slots(&el.children, out);
            }
            View::Element(el) if el.name.name.starts_with(char::is_uppercase) => {
                for c in &el.children {
                    match c {
                        View::Element(f) if f.name.name == "slot" && !f.args.is_empty() => {
                            find_slots(&f.children, out)
                        }
                        c => find_slots(std::slice::from_ref(c), out),
                    }
                }
            }
            View::Element(el) => find_slots(&el.children, out),
            View::If { then, els, .. } => {
                find_slots(then, out);
                if let Some(e) = els {
                    find_slots(e, out);
                }
            }
            View::For { body, .. } => find_slots(body, out),
            View::Match { arms, .. } => {
                for (_, b) in arms {
                    find_slots(b, out);
                }
            }
            View::Routes { arms, .. } => {
                for (_, _, b) in arms {
                    find_slots(b, out);
                }
            }
            _ => {}
        }
    }
}

/// The name in `slot(name)` (`""` for a plain `slot`) and the second argument, if any.
fn slot_args(el: &ast::Element) -> Result<(String, Option<&ast::Expr>)> {
    let name = |a: &ElemArg| match a {
        ElemArg::Expr(ast::Arg { name: None, value }) => match &value.kind {
            ast::ExprKind::Ident(n) => Ok(n.clone()),
            _ => Err(Diag::ty(
                value.span,
                "a slot is named with a plain name, e.g. `slot(header)`",
            )),
        },
        _ => Err(Diag::ty(
            el.span,
            "a slot is named with a plain name, e.g. `slot(header)`",
        )),
    };
    match el.args.as_slice() {
        [] => Ok((String::new(), None)),
        [a] => Ok((name(a)?, None)),
        [a, ElemArg::Expr(ast::Arg { name: None, value })] => Ok((name(a)?, Some(value))),
        _ => Err(Diag::ty(
            el.span,
            "`slot` takes a name and at most one value, e.g. `slot(row, r)`",
        )),
    }
}

/// Content given for one slot of a used component; `bind` holds the value the slot passes.
#[derive(Clone, Debug)]
pub struct TFill {
    pub name: String,
    pub bind: Option<LocalId>,
    pub views: Vec<TView>,
}
