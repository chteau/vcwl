//! Recursive-descent parser. See docs/language.md for the grammar.

use crate::ast::*;
use crate::diag::{Diag, Result, Span};
use crate::lexer::{StrPiece, Tok, Token};

const KEYWORDS: &[&str] = &[
    "fn",
    "let",
    "var",
    "if",
    "else",
    "while",
    "for",
    "in",
    "return",
    "match",
    "struct",
    "enum",
    "true",
    "false",
    "import",
    "export",
    "app",
    "component",
    "break",
    "continue",
];

pub struct Parser<'a> {
    toks: &'a [Token],
    pos: usize,
    next_id: &'a mut u32,
    /// Disallows `Name { ... }` struct literals (in `if`/`while`/`for`/`match` heads).
    no_struct: bool,
}

pub fn parse_file(path: &str, toks: &[Token], next_id: &mut u32) -> Result<File> {
    let mut p = Parser {
        toks,
        pos: 0,
        next_id,
        no_struct: false,
    };
    let mut items = Vec::new();
    while !p.at_eof() {
        if p.is_kw("impl") {
            items.extend(p.impl_block()?);
        } else {
            items.push(p.item()?);
        }
    }
    Ok(File {
        path: path.to_string(),
        items,
    })
}

impl<'a> Parser<'a> {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }
    fn token(&self) -> &Token {
        &self.toks[self.pos]
    }
    fn span(&self) -> Span {
        self.toks[self.pos].span
    }
    fn prev_span(&self) -> Span {
        self.toks[self.pos.saturating_sub(1)].span
    }
    fn at_eof(&self) -> bool {
        matches!(self.peek(), Tok::Eof)
    }
    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn is(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(q) if *q == p)
    }
    fn is_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == k)
    }
    fn eat(&mut self, p: &str) -> bool {
        if self.is(p) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn eat_kw(&mut self, k: &str) -> bool {
        if self.is_kw(k) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn describe(&self) -> String {
        match self.peek() {
            Tok::Ident(s) => format!("`{s}`"),
            Tok::Int(n) => format!("`{n}`"),
            Tok::Float(n) => format!("`{n}`"),
            Tok::Dim(n, u) => format!("`{n}{u}`"),
            Tok::Color(c) => format!("`#{c}`"),
            Tok::Str(_) => "a string".into(),
            Tok::Punct(p) => format!("`{p}`"),
            Tok::Eof => "end of file".into(),
        }
    }
    fn err<T>(&self, expected: &str) -> Result<T> {
        Err(Diag::syntax(
            self.span(),
            format!("expected {expected}, found {}", self.describe()),
        ))
    }
    fn expect(&mut self, p: &str) -> Result<Span> {
        if self.is(p) {
            Ok(self.bump().span)
        } else {
            self.err(&format!("`{p}`"))
        }
    }
    fn expect_kw(&mut self, k: &str) -> Result<Span> {
        if self.is_kw(k) {
            Ok(self.bump().span)
        } else {
            self.err(&format!("`{k}`"))
        }
    }
    fn ident(&mut self) -> Result<Ident> {
        match self.peek().clone() {
            Tok::Ident(name) if !KEYWORDS.contains(&name.as_str()) => {
                let span = self.bump().span;
                Ok(Ident { name, span })
            }
            Tok::Ident(name) => Err(Diag::syntax(
                self.span(),
                format!("`{name}` is a reserved keyword and cannot be used as a name"),
            )),
            _ => self.err("a name"),
        }
    }
    fn mk(&mut self, kind: ExprKind, span: Span) -> Expr {
        *self.next_id += 1;
        Expr {
            id: *self.next_id,
            kind,
            span,
        }
    }
    /// A statement must end with `;`, a newline, or the closing `}`.
    fn stmt_end(&mut self) -> Result<()> {
        if self.eat(";") || self.is("}") || self.token().nl || self.at_eof() {
            Ok(())
        } else {
            Err(Diag::syntax(
                self.span(),
                format!("expected end of statement, found {}", self.describe()),
            )
            .help("put each statement on its own line or separate them with `;`"))
        }
    }

    // ----- items -----

    /// A string literal without interpolation.
    fn plain_str(&mut self, what: &str) -> Result<String> {
        let Tok::Str(pieces) = self.peek().clone() else {
            return self.err(&format!("{what} string"));
        };
        let s = match pieces.as_slice() {
            [] => String::new(),
            [StrPiece::Lit(s)] => s.clone(),
            _ => {
                return Err(Diag::syntax(
                    self.span(),
                    format!("{what}s cannot contain interpolation"),
                ));
            }
        };
        self.bump();
        Ok(s)
    }

    fn item(&mut self) -> Result<Item> {
        let start = self.span();
        if self.eat_kw("import") {
            let path = self.plain_str("an import path")?;
            return Ok(Item::Import(Import {
                path,
                span: start.to(self.prev_span()),
            }));
        }
        if self.is_kw("font") && matches!(self.peek_at(1), Tok::Str(_)) {
            self.bump();
            let family = self.plain_str("a font family")?;
            if !self.eat_kw("from") {
                return self.err("`from \"path/to/font.woff2\"` or `from google`");
            }
            let google = self.eat_kw("google");
            let src = if google {
                String::new()
            } else {
                self.plain_str("a font file path")?
            };
            let (mut weight, mut italic) = (None, false);
            while !self.token().nl && !matches!(self.peek(), Tok::Eof) {
                if self.eat_kw("weight") {
                    let Tok::Int(lo) = *self.peek() else {
                        return self.err("a font weight (100–900)");
                    };
                    self.bump();
                    let hi = if self.eat("..") {
                        let Tok::Int(hi) = *self.peek() else {
                            return self.err("the upper font weight");
                        };
                        self.bump();
                        hi
                    } else {
                        lo
                    };
                    weight = Some((lo, hi));
                } else if self.eat_kw("italic") {
                    italic = true;
                } else {
                    return self.err("`weight`, `italic` or the end of the line");
                }
            }
            return Ok(Item::Font(FontDecl {
                family,
                src,
                google,
                weight,
                italic,
                span: start.to(self.prev_span()),
            }));
        }
        if self.is_kw("app") {
            return self.app().map(Item::App);
        }
        let exported = self.eat_kw("export");
        if self.is_kw("server") && matches!(self.peek_at(1), Tok::Ident(s) if s == "fn") {
            self.bump();
            let mut f = self.func(exported, false)?;
            f.server = true;
            Ok(Item::Fn(f))
        } else if self.is_kw("extern") && matches!(self.peek_at(1), Tok::Ident(s) if s == "fn") {
            // `extern fn name(params) -> Ret`: a JavaScript function, declared without a body.
            let start = self.bump().span;
            self.expect_kw("fn")?;
            let name = self.ident()?;
            self.expect("(")?;
            let params = self.params(false)?;
            let ret = if self.eat("->") {
                Some(self.type_expr()?)
            } else {
                None
            };
            let span = start.to(self.prev_span());
            Ok(Item::Fn(FnDecl {
                name,
                exported,
                server: false,
                ext: true,
                generics: Vec::new(),
                params,
                ret,
                body: Block {
                    stmts: Vec::new(),
                    tail: None,
                    span,
                },
                span,
            }))
        } else if self.is_kw("fn") {
            self.func(exported, false).map(Item::Fn)
        } else if self.eat_kw("interface") {
            self.interface(start, exported)
        } else if self.eat_kw("struct") {
            let name = self.ident()?;
            let generics = self.generic_params()?;
            self.expect("{")?;
            let mut fields = Vec::new();
            while !self.is("}") {
                let f = self.ident()?;
                self.expect(":")?;
                fields.push((f, self.type_expr()?));
                if !self.eat(",") && !self.is("}") && !self.token().nl {
                    return self.err("`,` or `}`");
                }
            }
            self.expect("}")?;
            Ok(Item::Struct(StructDecl {
                name,
                exported,
                generics,
                fields,
                interface: false,
                span: start.to(self.prev_span()),
            }))
        } else if self.eat_kw("enum") {
            let name = self.ident()?;
            let generics = self.generic_params()?;
            self.expect("{")?;
            let mut variants = Vec::new();
            while !self.is("}") {
                let vname = self.ident()?;
                let mut fields = Vec::new();
                if self.eat("(") {
                    while !self.is(")") {
                        fields.push(self.type_expr()?);
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect(")")?;
                }
                variants.push(Variant {
                    name: vname,
                    fields,
                });
                if !self.eat(",") && !self.is("}") && !self.token().nl {
                    return self.err("`,` or `}`");
                }
            }
            self.expect("}")?;
            Ok(Item::Enum(EnumDecl {
                name,
                exported,
                generics,
                variants,
                span: start.to(self.prev_span()),
            }))
        } else if self.eat_kw("component") {
            let name = self.ident()?;
            let params = if self.eat("(") {
                self.params(true)?
            } else {
                Vec::new()
            };
            self.expect("{")?;
            let body = self.views()?;
            Ok(Item::Component(Component {
                name,
                exported,
                params,
                body,
                span: start.to(self.prev_span()),
            }))
        } else if self.eat_kw("style") {
            let name = self.ident()?;
            let block = self.style_block()?;
            Ok(Item::Style(NamedStyle {
                name,
                exported,
                block,
            }))
        } else {
            self.err("an item (`fn`, `struct`, `enum`, `interface`, `impl`, `component`, `style`, `import` or `app`)")
        }
    }

    fn app(&mut self) -> Result<AppDecl> {
        let start = self.expect_kw("app")?;
        let title = match self.peek().clone() {
            Tok::Str(p) => match p.as_slice() {
                [StrPiece::Lit(s)] => {
                    self.bump();
                    s.clone()
                }
                _ => {
                    return Err(Diag::syntax(
                        self.span(),
                        "the app title cannot contain interpolation",
                    ));
                }
            },
            _ => return self.err("the app title string"),
        };
        let mut capabilities = Vec::new();
        if self.is("{") && !self.token().nl {
            self.bump();
            while !self.eat("}") {
                self.expect_kw("capabilities").map_err(|d| {
                    d.help("the app block currently only accepts `capabilities { ... }`")
                })?;
                self.expect("{")?;
                while !self.eat("}") {
                    let name = self.ident()?;
                    self.expect(":")?;
                    let (toks, span) = self.raw_value()?;
                    capabilities.push((name, toks, span));
                }
            }
        }
        Ok(AppDecl {
            title,
            capabilities,
            span: start.to(self.prev_span()),
        })
    }

    /// `impl [Interface for] Type[<T, ...>] { fn ... }`: the `Item::Impl` followed by its methods,
    /// named `Type.method`, with the impl's type parameters first.
    fn impl_block(&mut self) -> Result<Vec<Item>> {
        let start = self.bump().span;
        let first = self.ident()?;
        let (iface, ty) = if self.eat_kw("for") {
            (Some(first), self.ident()?)
        } else {
            (None, first)
        };
        let generics = self.generic_params()?;
        let span = start.to(self.prev_span());
        self.expect("{")?;
        let mut items = vec![Item::Impl(ImplDecl {
            ty: ty.clone(),
            iface,
            span,
        })];
        while !self.is("}") {
            let mut f = self.func(true, true)?;
            f.name.name = format!("{}.{}", ty.name, f.name.name);
            f.generics = generics.iter().cloned().chain(f.generics).collect();
            items.push(Item::Fn(f));
        }
        self.expect("}")?;
        Ok(items)
    }

    /// `interface Name { fn m(self, a: A) -> R ... }`: a struct with one `fn(A) -> R` field per method.
    fn interface(&mut self, start: Span, exported: bool) -> Result<Item> {
        let name = self.ident()?;
        self.expect("{")?;
        let mut fields = Vec::new();
        while !self.is("}") {
            let fstart = self.expect_kw("fn")?;
            let m = self.ident()?;
            self.expect("(")?;
            let params = self.params_self(true)?;
            let ret = if self.eat("->") {
                Some(Box::new(self.type_expr()?))
            } else {
                None
            };
            let ty = TypeExpr {
                kind: TypeExprKind::Fn(params.into_iter().skip(1).map(|p| p.ty).collect(), ret),
                span: fstart.to(self.prev_span()),
            };
            fields.push((m, ty));
            self.eat(",");
            self.eat(";");
        }
        self.expect("}")?;
        Ok(Item::Struct(StructDecl {
            name,
            exported,
            generics: Vec::new(),
            fields,
            interface: true,
            span: start.to(self.prev_span()),
        }))
    }

    /// `method`: the first parameter may be a bare `self` (of type `Self`).
    fn func(&mut self, exported: bool, method: bool) -> Result<FnDecl> {
        let start = self.expect_kw("fn")?;
        let name = self.ident()?;
        let generics = self.generic_params()?;
        self.expect("(")?;
        let params = if method {
            self.params_self(false)?
        } else {
            self.params(false)?
        };
        let ret = if self.eat("->") {
            Some(self.type_expr()?)
        } else {
            None
        };
        let body = self.block()?;
        Ok(FnDecl {
            name,
            exported,
            server: false,
            ext: false,
            generics,
            params,
            ret,
            body,
            span: start.to(self.prev_span()),
        })
    }

    fn generic_params(&mut self) -> Result<Vec<Ident>> {
        let mut out = Vec::new();
        if self.eat("<") {
            while !self.is(">") {
                out.push(self.ident()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(">")?;
        }
        Ok(out)
    }

    /// Parameter list of a method; `required` demands the leading `self`.
    fn params_self(&mut self, required: bool) -> Result<Vec<Param>> {
        let mut out = Vec::new();
        if matches!(self.peek(), Tok::Ident(s) if s == "self")
            && !matches!(self.peek_at(1), Tok::Punct(":"))
        {
            let name = self.ident()?;
            let ty = TypeExpr {
                kind: TypeExprKind::Named("Self".into(), Vec::new()),
                span: name.span,
            };
            out.push(Param {
                name,
                ty,
                default: None,
            });
            if !self.eat(",") && !self.is(")") {
                return self.err("`,` or `)`");
            }
        } else if required {
            return self.err("`self` (interface methods take `self` first)");
        }
        out.extend(self.params(false)?);
        Ok(out)
    }

    /// Parameter list after `(`; consumes the closing `)`.
    fn params(&mut self, defaults: bool) -> Result<Vec<Param>> {
        let mut out = Vec::new();
        while !self.is(")") {
            let name = self.ident()?;
            self.expect(":")
                .map_err(|d| d.help("parameters need a type, e.g. `count: Int`"))?;
            let ty = self.type_expr()?;
            let default = if defaults && self.eat("=") {
                Some(self.expr()?)
            } else {
                None
            };
            out.push(Param { name, ty, default });
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        Ok(out)
    }

    fn type_expr(&mut self) -> Result<TypeExpr> {
        let start = self.span();
        let mut ty = if self.eat("[") {
            let inner = self.type_expr()?;
            self.expect("]")?;
            TypeExpr {
                kind: TypeExprKind::Array(Box::new(inner)),
                span: start.to(self.prev_span()),
            }
        } else if self.eat("(") {
            if self.eat(")") {
                TypeExpr {
                    kind: TypeExprKind::Unit,
                    span: start.to(self.prev_span()),
                }
            } else {
                let mut items = vec![self.type_expr()?];
                let mut comma = false;
                while self.eat(",") {
                    comma = true;
                    if self.is(")") {
                        break;
                    }
                    items.push(self.type_expr()?);
                }
                self.expect(")")?;
                if items.len() == 1 && !comma {
                    items.remove(0)
                } else if (2..=4).contains(&items.len()) {
                    TypeExpr {
                        kind: TypeExprKind::Named(format!("Tuple{}", items.len()), items),
                        span: start.to(self.prev_span()),
                    }
                } else {
                    return Err(Diag::syntax(
                        start.to(self.prev_span()),
                        "tuple types have 2 to 4 elements",
                    ));
                }
            }
        } else if self.eat_kw("fn") {
            self.expect("(")?;
            let mut params = Vec::new();
            while !self.is(")") {
                params.push(self.type_expr()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
            let ret = if self.eat("->") {
                Some(Box::new(self.type_expr()?))
            } else {
                None
            };
            return Ok(TypeExpr {
                kind: TypeExprKind::Fn(params, ret),
                span: start.to(self.prev_span()),
            });
        } else {
            let name = self.ident()?;
            let mut args = Vec::new();
            if self.eat("<") {
                while !self.is(">") {
                    args.push(self.type_expr()?);
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(">")?;
            }
            TypeExpr {
                kind: TypeExprKind::Named(name.name, args),
                span: start.to(self.prev_span()),
            }
        };
        while self.is("?") && !self.token().nl {
            self.bump();
            ty = TypeExpr {
                span: start.to(self.prev_span()),
                kind: TypeExprKind::Optional(Box::new(ty)),
            };
        }
        Ok(ty)
    }

    // ----- statements -----

    pub fn block(&mut self) -> Result<Block> {
        let start = self.expect("{")?;
        let mut stmts = Vec::new();
        let mut tail = None;
        while !self.is("}") {
            if self.at_eof() {
                return Err(Diag::syntax(start, "unclosed `{`"));
            }
            let mut batch = self.stmts()?;
            if self.is("}")
                && !self.prev_is(";")
                && let Some(Stmt::Expr(_)) = batch.last()
            {
                let Some(Stmt::Expr(e)) = batch.pop() else {
                    unreachable!()
                };
                tail = Some(Box::new(e));
            }
            stmts.extend(batch);
        }
        self.bump();
        Ok(Block {
            stmts,
            tail,
            span: start.to(self.prev_span()),
        })
    }

    fn stmts(&mut self) -> Result<Vec<Stmt>> {
        let start = self.span();
        if (self.is_kw("let") || self.is_kw("var")) && matches!(self.peek_at(1), Tok::Punct("(")) {
            let mutable = self.bump().tok == Tok::Ident("var".into());
            let pat = self.pattern()?;
            self.expect("=")
                .map_err(|d| d.help("bindings must be initialized, e.g. `let x = 0`"))?;
            let value = self.expr()?;
            self.stmt_end()?;
            let span = start.to(self.prev_span());
            let tmp = Ident {
                name: format!("_tup{}", pat.span.lo),
                span: pat.span,
            };
            let mut out = vec![Stmt::Let {
                name: tmp.clone(),
                mutable: false,
                ty: None,
                value,
                span,
            }];
            out.extend(self.destructure(&tmp, &pat, mutable, span));
            return Ok(out);
        }
        Ok(vec![self.stmt()?])
    }

    fn destructure(&mut self, tmp: &Ident, pat: &Pattern, mutable: bool, span: Span) -> Vec<Stmt> {
        let mut names = Vec::new();
        pat_binds(pat, &mut names);
        let mut out = Vec::new();
        for name in names {
            let scrut = self.mk(ExprKind::Ident(tmp.name.clone()), pat.span);
            let body = self.mk(ExprKind::Ident(name.clone()), pat.span);
            let value = self.mk(
                ExprKind::Match {
                    scrut: Box::new(scrut),
                    arms: vec![Arm {
                        pat: keep_bind(pat, &name),
                        body,
                    }],
                },
                pat.span,
            );
            out.push(Stmt::Let {
                name: Ident {
                    name,
                    span: pat.span,
                },
                mutable,
                ty: None,
                value,
                span,
            });
        }
        out
    }

    fn prev_is(&self, p: &str) -> bool {
        matches!(&self.toks[self.pos.saturating_sub(1)].tok, Tok::Punct(q) if *q == p)
    }

    fn stmt(&mut self) -> Result<Stmt> {
        let start = self.span();
        if self.is_kw("let") || self.is_kw("var") {
            let mutable = self.bump().tok == Tok::Ident("var".into());
            let name = self.ident()?;
            let ty = if self.eat(":") {
                Some(self.type_expr()?)
            } else {
                None
            };
            self.expect("=")
                .map_err(|d| d.help("bindings must be initialized, e.g. `let x = 0`"))?;
            let value = self.expr()?;
            self.stmt_end()?;
            return Ok(Stmt::Let {
                name,
                mutable,
                ty,
                value,
                span: start.to(self.prev_span()),
            });
        }
        if self.eat_kw("while") {
            let cond = self.expr_no_struct()?;
            let body = self.block()?;
            return Ok(Stmt::While {
                cond,
                body,
                span: start.to(self.prev_span()),
            });
        }
        if self.eat_kw("for") {
            let (var, pat) = if self.is("(") {
                let p = self.pattern()?;
                let tmp = Ident {
                    name: format!("_tup{}", p.span.lo),
                    span: p.span,
                };
                (tmp, Some(p))
            } else {
                (self.ident()?, None)
            };
            self.expect_kw("in")?;
            let iter = self.expr_no_struct()?;
            let mut body = self.block()?;
            if let Some(p) = pat {
                let mut stmts = self.destructure(&var, &p, false, p.span);
                stmts.append(&mut body.stmts);
                body.stmts = stmts;
            }
            return Ok(Stmt::For {
                var,
                iter,
                body,
                span: start.to(self.prev_span()),
            });
        }
        if self.eat_kw("break") {
            self.stmt_end()?;
            return Ok(Stmt::Break(start));
        }
        if self.eat_kw("continue") {
            self.stmt_end()?;
            return Ok(Stmt::Continue(start));
        }
        if self.is_kw("emit")
            && matches!(self.peek_at(1), Tok::Ident(_))
            && !self.toks[self.pos + 1].nl
        {
            self.bump();
            let name = self.ident()?;
            let value = if self.is("(") && !self.token().nl {
                self.bump();
                let v = self.expr()?;
                self.expect(")")?;
                Some(v)
            } else {
                None
            };
            self.stmt_end()?;
            return Ok(Stmt::Emit {
                name,
                value,
                span: start.to(self.prev_span()),
            });
        }
        if self.is_kw("fetch")
            && matches!(self.peek_at(1), Tok::Ident(_))
            && matches!(self.peek_at(2), Tok::Punct("("))
        {
            return self.fetch();
        }
        let e = self.expr()?;
        let op = match self.peek() {
            Tok::Punct("=") => Some(None),
            Tok::Punct("+=") => Some(Some(BinOp::Add)),
            Tok::Punct("-=") => Some(Some(BinOp::Sub)),
            Tok::Punct("*=") => Some(Some(BinOp::Mul)),
            Tok::Punct("/=") => Some(Some(BinOp::Div)),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let value = self.expr()?;
            self.stmt_end()?;
            return Ok(Stmt::Assign {
                target: e,
                op,
                value,
                span: start.to(self.prev_span()),
            });
        }
        if !matches!(
            e.kind,
            ExprKind::If { .. } | ExprKind::Match { .. } | ExprKind::Block(_)
        ) {
            self.stmt_end()?;
        }
        Ok(Stmt::Expr(e))
    }

    fn fetch(&mut self) -> Result<Stmt> {
        let start = self.expect_kw("fetch")?;
        let mut f = self.fetch_head(start)?;
        self.expect("{")?;
        f.arms = self.arms()?;
        f.span = start.to(self.prev_span());
        Ok(Stmt::Fetch(Box::new(f)))
    }

    /// `method(args) [as T]` after `fetch` or `await`.
    fn fetch_head(&mut self, start: Span) -> Result<Fetch> {
        let method = self.ident()?;
        self.expect("(")?;
        let mut args = Vec::new();
        while !self.is(")") {
            args.push(self.expr()?);
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        let ty = if self.eat_kw("as") {
            Some(self.type_expr()?)
        } else {
            None
        };
        Ok(Fetch {
            method,
            args,
            ty,
            arms: Vec::new(),
            span: start.to(self.prev_span()),
        })
    }

    // ----- expressions -----

    pub fn expr(&mut self) -> Result<Expr> {
        let saved = std::mem::replace(&mut self.no_struct, false);
        let r = self.binary(0);
        self.no_struct = saved;
        r
    }

    fn expr_no_struct(&mut self) -> Result<Expr> {
        let saved = std::mem::replace(&mut self.no_struct, true);
        let r = self.binary(0);
        self.no_struct = saved;
        r
    }

    fn binop(&self) -> Option<(BinOp, u8)> {
        if self.token().nl {
            return None;
        }
        let Tok::Punct(p) = self.peek() else {
            return None;
        };
        Some(match *p {
            "||" => (BinOp::Or, 1),
            "&&" => (BinOp::And, 2),
            "==" => (BinOp::Eq, 3),
            "!=" => (BinOp::Ne, 3),
            "<" => (BinOp::Lt, 3),
            "<=" => (BinOp::Le, 3),
            ">" => (BinOp::Gt, 3),
            ">=" => (BinOp::Ge, 3),
            "+" => (BinOp::Add, 5),
            "-" => (BinOp::Sub, 5),
            "*" => (BinOp::Mul, 6),
            "/" => (BinOp::Div, 6),
            "%" => (BinOp::Rem, 6),
            _ => return None,
        })
    }

    fn binary(&mut self, min: u8) -> Result<Expr> {
        let mut lhs = self.unary()?;
        loop {
            // `x |> f(a)` is `f(x, a)`; the lowest precedence, and it may start a line.
            if min == 0 && self.is("|>") {
                self.bump();
                let rhs = self.binary(1)?;
                let span = lhs.span.to(rhs.span);
                let kind = match rhs.kind {
                    ExprKind::Call {
                        callee,
                        type_args,
                        mut args,
                    } => {
                        args.insert(
                            0,
                            Arg {
                                name: None,
                                value: lhs,
                            },
                        );
                        ExprKind::Call {
                            callee,
                            type_args,
                            args,
                        }
                    }
                    ExprKind::Ident(name) => ExprKind::Call {
                        callee: Ident {
                            name,
                            span: rhs.span,
                        },
                        type_args: Vec::new(),
                        args: vec![Arg {
                            name: None,
                            value: lhs,
                        }],
                    },
                    _ => {
                        return Err(Diag::syntax(
                            rhs.span,
                            "the right side of `|>` must be a function name or call",
                        )
                        .help("e.g. `xs |> total` or `name |> greet(\"!\")`"));
                    }
                };
                lhs = self.mk(kind, span);
                continue;
            }
            if min <= 4 && self.is("..") && !self.token().nl {
                self.bump();
                let rhs = self.binary(5)?;
                let span = lhs.span.to(rhs.span);
                lhs = self.mk(ExprKind::Range(Box::new(lhs), Box::new(rhs)), span);
                continue;
            }
            let Some((op, prec)) = self.binop() else {
                break;
            };
            if prec < min {
                break;
            }
            let op_span = self.bump().span;
            let rhs = self.binary(prec + 1)?;
            if prec == 3 && self.binop().is_some_and(|(_, p)| p == 3) {
                return Err(
                    Diag::syntax(self.span(), "comparison operators cannot be chained")
                        .help("combine comparisons with `&&`, e.g. `a < b && b < c`"),
                );
            }
            let _ = op_span;
            let span = lhs.span.to(rhs.span);
            lhs = self.mk(ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), span);
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr> {
        let start = self.span();
        let op = if self.eat("-") {
            UnOp::Neg
        } else if self.eat("!") {
            UnOp::Not
        } else {
            return self.postfix();
        };
        let e = self.unary()?;
        let span = start.to(e.span);
        Ok(match (op, &e.kind) {
            (UnOp::Neg, ExprKind::Int(n)) => self.mk(ExprKind::Int(n.wrapping_neg()), span),
            (UnOp::Neg, ExprKind::Float(n)) => self.mk(ExprKind::Float(-n), span),
            _ => self.mk(ExprKind::Unary(op, Box::new(e)), span),
        })
    }

    fn postfix(&mut self) -> Result<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.is(".") {
                self.bump();
                if let Tok::Int(n @ 0..=3) = *self.peek() {
                    let sp = self.bump().span;
                    let name = Ident {
                        name: format!("_{n}"),
                        span: sp,
                    };
                    let span = e.span.to(sp);
                    e = self.mk(
                        ExprKind::Method {
                            recv: Box::new(e),
                            name,
                            type_args: Vec::new(),
                            args: Vec::new(),
                        },
                        span,
                    );
                    continue;
                }
                let name = self.ident()?;
                if self.is("(") && !self.token().nl {
                    self.bump();
                    let args = self.call_args()?;
                    let span = e.span.to(self.prev_span());
                    e = self.mk(
                        ExprKind::Method {
                            recv: Box::new(e),
                            name,
                            type_args: Vec::new(),
                            args,
                        },
                        span,
                    );
                } else if self.is("<")
                    && !self.token().nl
                    && let Some(type_args) = self.try_type_args()
                {
                    self.expect("(")?;
                    let args = self.call_args()?;
                    let span = e.span.to(self.prev_span());
                    e = self.mk(
                        ExprKind::Method {
                            recv: Box::new(e),
                            name,
                            type_args,
                            args,
                        },
                        span,
                    );
                } else {
                    let span = e.span.to(name.span);
                    e = self.mk(ExprKind::Field(Box::new(e), name), span);
                }
            } else if self.is("(") && !self.token().nl {
                self.bump();
                let args = self.call_args()?;
                let span = e.span.to(self.prev_span());
                e = self.mk(ExprKind::Apply(Box::new(e), args), span);
            } else if self.is("[") && !self.token().nl {
                self.bump();
                let idx = self.expr()?;
                self.expect("]")?;
                let span = e.span.to(self.prev_span());
                e = self.mk(ExprKind::Index(Box::new(e), Box::new(idx)), span);
            } else if self.is("?") && !self.token().nl {
                self.bump();
                let span = e.span.to(self.prev_span());
                e = self.mk(ExprKind::Try(Box::new(e)), span);
            } else {
                return Ok(e);
            }
        }
    }

    /// Speculatively parses `<T, U>` followed by `(`; restores position on failure.
    fn try_type_args(&mut self) -> Option<Vec<TypeExpr>> {
        let save = self.pos;
        self.bump();
        let mut args = Vec::new();
        while let Ok(t) = self.type_expr() {
            args.push(t);
            if !self.eat(",") {
                break;
            }
        }
        if !args.is_empty() && self.eat(">") && self.is("(") {
            return Some(args);
        }
        self.pos = save;
        None
    }

    fn call_args(&mut self) -> Result<Vec<Arg>> {
        let mut args = Vec::new();
        while !self.is(")") {
            let name = if matches!(self.peek(), Tok::Ident(_))
                && matches!(self.peek_at(1), Tok::Punct(":"))
            {
                let n = self.ident()?;
                self.bump();
                Some(n)
            } else {
                None
            };
            args.push(Arg {
                name,
                value: self.expr()?,
            });
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        Ok(args)
    }

    fn primary(&mut self) -> Result<Expr> {
        let t = self.token().clone();
        let span = t.span;
        match t.tok {
            Tok::Int(n) => {
                self.bump();
                Ok(self.mk(ExprKind::Int(n), span))
            }
            Tok::Float(n) => {
                self.bump();
                Ok(self.mk(ExprKind::Float(n), span))
            }
            Tok::Dim(n, u) => {
                self.bump();
                Ok(self.mk(ExprKind::Dim(n, u), span))
            }
            Tok::Color(c) => {
                self.bump();
                Ok(self.mk(ExprKind::Color(c), span))
            }
            Tok::Str(pieces) => {
                self.bump();
                let mut parts = Vec::new();
                for piece in pieces {
                    match piece {
                        StrPiece::Lit(s) => parts.push(StrPart::Lit(s)),
                        StrPiece::Expr(toks, ispan) => {
                            let mut sub = Parser {
                                toks: &toks,
                                pos: 0,
                                next_id: self.next_id,
                                no_struct: false,
                            };
                            let e = sub.expr()?;
                            if !sub.at_eof() {
                                return Err(Diag::syntax(
                                    sub.span(),
                                    format!("unexpected {} in interpolation", sub.describe()),
                                ));
                            }
                            let _ = ispan;
                            parts.push(StrPart::Expr(e));
                        }
                    }
                }
                Ok(self.mk(ExprKind::Str(parts), span))
            }
            Tok::Punct("(") => {
                self.bump();
                if self.eat(")") {
                    return Ok(self.mk(ExprKind::Unit, span.to(self.prev_span())));
                }
                let mut e = self.expr()?;
                if self.is(",") {
                    let mut items = vec![e];
                    while self.eat(",") {
                        if self.is(")") {
                            break;
                        }
                        items.push(self.expr()?);
                    }
                    self.expect(")")?;
                    let full = span.to(self.prev_span());
                    if !(2..=4).contains(&items.len()) {
                        return Err(Diag::syntax(full, "tuples have 2 to 4 elements"));
                    }
                    let callee = Ident {
                        name: format!("Tuple{}", items.len()),
                        span,
                    };
                    let args = items
                        .into_iter()
                        .map(|value| Arg { name: None, value })
                        .collect();
                    return Ok(self.mk(
                        ExprKind::Call {
                            callee,
                            type_args: Vec::new(),
                            args,
                        },
                        full,
                    ));
                }
                self.expect(")")?;
                e.span = span.to(self.prev_span());
                Ok(e)
            }
            Tok::Punct("|" | "||") => self.lambda(),
            Tok::Ident(ref k)
                if k == "await"
                    && matches!(self.peek_at(1), Tok::Ident(_))
                    && matches!(self.peek_at(2), Tok::Punct("(")) =>
            {
                self.bump();
                let f = self.fetch_head(span)?;
                let s = f.span;
                Ok(self.mk(ExprKind::Await(Box::new(f)), s))
            }
            Tok::Punct("[") => {
                self.bump();
                let mut items = Vec::new();
                while !self.is("]") {
                    items.push(self.expr()?);
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect("]")?;
                Ok(self.mk(ExprKind::Array(items), span.to(self.prev_span())))
            }
            Tok::Punct("{") => {
                let b = self.block()?;
                let s = b.span;
                Ok(self.mk(ExprKind::Block(b), s))
            }
            Tok::Ident(ref k) if k == "true" || k == "false" => {
                self.bump();
                Ok(self.mk(ExprKind::Bool(k == "true"), span))
            }
            Tok::Ident(ref k) if k == "if" => self.if_expr(),
            Tok::Ident(ref k) if k == "return" => {
                self.bump();
                let value = if self.is("}") || self.is(";") || self.is(",") || self.token().nl {
                    None
                } else {
                    Some(Box::new(self.expr()?))
                };
                let span = span.to(self.prev_span());
                Ok(self.mk(ExprKind::Return(value), span))
            }
            Tok::Ident(ref k) if k == "match" => {
                self.bump();
                let scrut = self.expr_no_struct()?;
                self.expect("{")?;
                let arms = self.arms()?;
                Ok(self.mk(
                    ExprKind::Match {
                        scrut: Box::new(scrut),
                        arms,
                    },
                    span.to(self.prev_span()),
                ))
            }
            Tok::Ident(_) => {
                let name = self.ident()?;
                if self.is("(") && !self.token().nl {
                    self.bump();
                    let args = self.call_args()?;
                    return Ok(self.mk(
                        ExprKind::Call {
                            callee: name,
                            type_args: Vec::new(),
                            args,
                        },
                        span.to(self.prev_span()),
                    ));
                }
                if self.is("<")
                    && !self.token().nl
                    && let Some(type_args) = self.try_type_args()
                {
                    self.expect("(")?;
                    let args = self.call_args()?;
                    return Ok(self.mk(
                        ExprKind::Call {
                            callee: name,
                            type_args,
                            args,
                        },
                        span.to(self.prev_span()),
                    ));
                }
                if self.is("{") && !self.no_struct && name.name.starts_with(char::is_uppercase) {
                    return self.struct_lit(name);
                }
                Ok(self.mk(ExprKind::Ident(name.name), span))
            }
            _ => self.err("an expression"),
        }
    }

    /// `|a, b: T| expr` or `|a| -> R { ... }`.
    fn lambda(&mut self) -> Result<Expr> {
        let start = self.span();
        let mut params = Vec::new();
        if !self.eat("||") {
            self.expect("|")?;
            while !self.is("|") {
                let name = self.ident()?;
                let ty = if self.eat(":") {
                    Some(self.type_expr()?)
                } else {
                    None
                };
                params.push((name, ty));
                if !self.eat(",") {
                    break;
                }
            }
            self.expect("|")?;
        }
        let (ret, body) = if self.eat("->") {
            let ret = self.type_expr()?;
            let b = self.block()?;
            let s = b.span;
            (Some(ret), self.mk(ExprKind::Block(b), s))
        } else {
            let saved = std::mem::replace(&mut self.no_struct, false);
            let body = self.expr();
            self.no_struct = saved;
            (None, body?)
        };
        let span = start.to(body.span);
        Ok(self.mk(
            ExprKind::Lambda {
                params,
                ret,
                body: Box::new(body),
            },
            span,
        ))
    }

    fn struct_lit(&mut self, name: Ident) -> Result<Expr> {
        self.expect("{")?;
        let mut fields = Vec::new();
        let mut base = None;
        while !self.is("}") {
            if self.eat("..") {
                base = Some(Box::new(self.expr()?));
            } else {
                let f = self.ident()?;
                let value = if self.eat(":") {
                    self.expr()?
                } else {
                    // Shorthand `Point { x, y }`.
                    self.mk(ExprKind::Ident(f.name.clone()), f.span)
                };
                fields.push((f, value));
            }
            if !self.eat(",") && !self.is("}") && !self.token().nl {
                return self.err("`,` or `}`");
            }
        }
        self.expect("}")?;
        let span = name.span.to(self.prev_span());
        Ok(self.mk(ExprKind::StructLit { name, fields, base }, span))
    }

    fn if_expr(&mut self) -> Result<Expr> {
        let start = self.expect_kw("if")?;
        let cond = self.expr_no_struct()?;
        let then = self.block()?;
        let els = if self.eat_kw("else") {
            if self.is_kw("if") {
                Some(Box::new(self.if_expr()?))
            } else {
                let b = self.block()?;
                let s = b.span;
                Some(Box::new(self.mk(ExprKind::Block(b), s)))
            }
        } else {
            None
        };
        Ok(self.mk(
            ExprKind::If {
                cond: Box::new(cond),
                then,
                els,
            },
            start.to(self.prev_span()),
        ))
    }

    /// Match arms after `{`; consumes the closing `}`.
    fn arms(&mut self) -> Result<Vec<Arm>> {
        let mut arms = Vec::new();
        while !self.eat("}") {
            let pat = self.pattern()?;
            self.expect("=>")?;
            let body = self.expr()?;
            arms.push(Arm { pat, body });
            if !self.eat(",") && !self.is("}") && !self.token().nl {
                return self.err("`,` or `}` after match arm");
            }
        }
        Ok(arms)
    }

    fn pattern(&mut self) -> Result<Pattern> {
        let start = self.span();
        let kind = match self.peek().clone() {
            Tok::Int(n) => {
                self.bump();
                PatKind::Int(n)
            }
            Tok::Punct("-") if matches!(self.peek_at(1), Tok::Int(_)) => {
                self.bump();
                let Tok::Int(n) = self.bump().tok else {
                    unreachable!()
                };
                PatKind::Int(-n)
            }
            Tok::Str(p) => {
                self.bump();
                match p.as_slice() {
                    [StrPiece::Lit(s)] => PatKind::Str(s.clone()),
                    _ => {
                        return Err(Diag::syntax(
                            start,
                            "string patterns cannot contain interpolation",
                        ));
                    }
                }
            }
            Tok::Punct("(") => {
                self.bump();
                let mut items = Vec::new();
                let mut comma = false;
                while !self.is(")") {
                    items.push(self.pattern()?);
                    if !self.eat(",") {
                        break;
                    }
                    comma = true;
                }
                self.expect(")")?;
                let span = start.to(self.prev_span());
                if items.len() == 1 && !comma {
                    return Ok(items.remove(0));
                }
                if !(2..=4).contains(&items.len()) {
                    return Err(Diag::syntax(span, "tuple patterns have 2 to 4 elements"));
                }
                PatKind::Variant {
                    qual: None,
                    name: Ident {
                        name: format!("Tuple{}", items.len()),
                        span,
                    },
                    args: items,
                }
            }
            Tok::Ident(s) if s == "_" => {
                self.bump();
                PatKind::Wild
            }
            Tok::Ident(s) if s == "true" || s == "false" => {
                self.bump();
                PatKind::Bool(s == "true")
            }
            Tok::Ident(_) => {
                let first = self.ident()?;
                let (qual, name) = if self.eat(".") {
                    (Some(first), self.ident()?)
                } else {
                    (None, first)
                };
                if qual.is_none() && !name.name.starts_with(char::is_uppercase) && !self.is("(") {
                    PatKind::Bind(name.name)
                } else {
                    let mut args = Vec::new();
                    if self.eat("(") {
                        while !self.is(")") {
                            args.push(self.pattern()?);
                            if !self.eat(",") {
                                break;
                            }
                        }
                        self.expect(")")?;
                    }
                    PatKind::Variant { qual, name, args }
                }
            }
            _ => return self.err("a pattern"),
        };
        Ok(Pattern {
            kind,
            span: start.to(self.prev_span()),
        })
    }

    // ----- views -----

    /// View items after `{`; consumes the closing `}`.
    fn views(&mut self) -> Result<Vec<View>> {
        let open = self.prev_span();
        let mut out = Vec::new();
        while !self.eat("}") {
            if self.at_eof() {
                return Err(Diag::syntax(open, "unclosed `{`"));
            }
            out.push(self.view()?);
        }
        Ok(out)
    }

    fn view(&mut self) -> Result<View> {
        let start = self.span();
        if self.eat_kw("state") {
            let name = self.ident()?;
            let ty = if self.eat(":") {
                Some(self.type_expr()?)
            } else {
                None
            };
            self.expect("=")
                .map_err(|d| d.help("state needs an initial value, e.g. `state count: Int = 0`"))?;
            let init = self.expr()?;
            self.stmt_end()?;
            return Ok(View::State {
                name,
                ty,
                init,
                span: start.to(self.prev_span()),
            });
        }
        if self.eat_kw("let") {
            let name = self.ident()?;
            let ty = if self.eat(":") {
                Some(self.type_expr()?)
            } else {
                None
            };
            self.expect("=")?;
            let value = self.expr()?;
            self.stmt_end()?;
            return Ok(View::Let {
                name,
                ty,
                value,
                span: start.to(self.prev_span()),
            });
        }
        if self.is_kw("var") {
            return Err(
                Diag::syntax(start, "`var` is not allowed in a component body")
                    .help("use `state name: Type = value` for mutable component data"),
            );
        }
        if self.is_kw("if") {
            return self.view_if();
        }
        if self.eat_kw("for") {
            let var = self.ident()?;
            let index = if self.eat(",") {
                Some(self.ident()?)
            } else {
                None
            };
            self.expect_kw("in")?;
            let iter = self.expr_no_struct()?;
            let key = if self.eat_kw("key") {
                Some(self.expr_no_struct()?)
            } else {
                None
            };
            self.expect("{")?;
            let body = self.views()?;
            return Ok(View::For {
                var,
                index,
                iter,
                key,
                body,
                span: start.to(self.prev_span()),
            });
        }
        if self.eat_kw("match") {
            let scrut = self.expr_no_struct()?;
            self.expect("{")?;
            let mut arms = Vec::new();
            while !self.eat("}") {
                let pat = self.pattern()?;
                self.expect("=>")?;
                arms.push((pat, self.view_arm_body()?));
                self.eat(",");
            }
            return Ok(View::Match {
                scrut,
                arms,
                span: start.to(self.prev_span()),
            });
        }
        if self.is_kw("event") && matches!(self.peek_at(1), Tok::Ident(_)) {
            self.bump();
            let name = self.ident()?;
            let ty = if self.eat("(") {
                let t = self.type_expr()?;
                self.expect(")")?;
                Some(t)
            } else {
                None
            };
            self.stmt_end()?;
            return Ok(View::Event {
                name,
                ty,
                span: start.to(self.prev_span()),
            });
        }
        // `slot name(Type)` declares a slot with a value; `slot` alone or `slot(...)` is a view.
        if self.is_kw("slot")
            && matches!(self.peek_at(1), Tok::Ident(_))
            && !self.toks[self.pos + 1].nl
        {
            self.bump();
            let name = self.ident()?;
            self.expect("(")?;
            let ty = self.type_expr()?;
            self.expect(")")?;
            self.stmt_end()?;
            return Ok(View::SlotDecl {
                name,
                ty,
                span: start.to(self.prev_span()),
            });
        }
        if self.is_kw("on") && matches!(self.peek_at(1), Tok::Ident(_)) {
            self.bump();
            let event = self.ident()?;
            let param = if self.eat("(") {
                let p = self.ident()?;
                self.expect(")")?;
                Some(p)
            } else {
                None
            };
            let body = self.block()?;
            return Ok(View::On {
                event,
                param,
                body,
                span: start.to(self.prev_span()),
            });
        }
        if self.is_kw("style") && !matches!(self.peek_at(1), Tok::Punct("(")) {
            self.bump();
            if self.eat_kw("if") {
                let cond = self.expr_no_struct()?;
                let block = self.style_block()?;
                return Ok(View::StyleIf {
                    cond,
                    block,
                    span: start.to(self.prev_span()),
                });
            }
            if self.is("{") {
                return Ok(View::Style(self.style_block()?));
            }
            if self.eat_kw("bind") {
                let prop = self.prop_name()?;
                self.expect(":")?;
                let (mut toks, vspan) = self.raw_value()?;
                // A trailing unit (`%`, `px`, ...) is not part of the expression.
                let unit = match &toks.last().unwrap().tok {
                    Tok::Punct("%") => Some("%".to_string()),
                    Tok::Ident(u) if ["px", "rem", "em", "vw", "vh"].contains(&u.as_str()) => {
                        Some(u.clone())
                    }
                    _ => None,
                };
                if unit.is_some() {
                    toks.pop();
                }
                let end = toks.last().map_or(vspan, |t| t.span);
                toks.push(Token {
                    tok: Tok::Eof,
                    span: end,
                    nl: true,
                });
                let mut sub = Parser {
                    toks: &toks,
                    pos: 0,
                    next_id: self.next_id,
                    no_struct: false,
                };
                let value = sub.expr()?;
                if !sub.at_eof() {
                    return Err(Diag::syntax(sub.span(), "expected the end of the value"));
                }
                return Ok(View::StyleBind {
                    prop,
                    value,
                    unit,
                    span: start.to(vspan),
                });
            }
            return Ok(View::StyleRef(self.ident()?));
        }
        if self.is_kw("routes") && matches!(self.peek_at(1), Tok::Punct("{")) {
            self.bump();
            self.bump();
            let mut arms = Vec::new();
            while !self.eat("}") {
                let aspan = self.span();
                let path = match self.peek().clone() {
                    Tok::Str(p) => match p.as_slice() {
                        [StrPiece::Lit(s)] => {
                            self.bump();
                            Some(s.clone())
                        }
                        _ => {
                            return Err(Diag::syntax(
                                aspan,
                                "route paths cannot contain interpolation",
                            ));
                        }
                    },
                    Tok::Ident(s) if s == "_" => {
                        self.bump();
                        None
                    }
                    _ => return self.err("a route path string or `_`"),
                };
                self.expect("=>")?;
                arms.push((path, aspan, self.view_arm_body()?));
                self.eat(",");
            }
            return Ok(View::Routes {
                arms,
                span: start.to(self.prev_span()),
            });
        }
        self.element().map(View::Element)
    }

    fn view_arm_body(&mut self) -> Result<Vec<View>> {
        if self.eat("{") {
            self.views()
        } else {
            Ok(vec![self.view()?])
        }
    }

    fn view_if(&mut self) -> Result<View> {
        let start = self.expect_kw("if")?;
        let cond = self.expr_no_struct()?;
        self.expect("{")?;
        let then = self.views()?;
        let els = if self.eat_kw("else") {
            if self.is_kw("if") {
                Some(vec![self.view_if()?])
            } else {
                self.expect("{")?;
                Some(self.views()?)
            }
        } else {
            None
        };
        Ok(View::If {
            cond,
            then,
            els,
            span: start.to(self.prev_span()),
        })
    }

    fn element(&mut self) -> Result<Element> {
        let name = self.ident().map_err(|d| {
            Diag::syntax(d.span, format!("expected a view item, found {}", self.describe()))
                .help("component bodies contain `state`, `let`, elements like `text(...)`, `if`, `for`, `on` and `style`")
        })?;
        if ["=", "+=", "-=", "*=", "/="].iter().any(|op| self.is(op)) && !self.token().nl {
            return Err(Diag::syntax(self.token().span, format!("cannot assign to `{}` while rendering", name.name))
                .help("views must not change state; assign inside an event handler such as `on click { ... }`"));
        }
        let mut args = Vec::new();
        if self.is("(") && !self.token().nl {
            self.bump();
            while !self.is(")") {
                if let (Tok::Ident(n), Tok::Punct(":")) =
                    (self.peek().clone(), self.peek_at(1).clone())
                    && crate::style::is_style_arg(&n)
                {
                    let id = self.ident()?;
                    self.bump();
                    let (toks, span) = self.raw_until_comma()?;
                    args.push(ElemArg::Style(id, toks, span));
                } else {
                    let name = if matches!(self.peek(), Tok::Ident(_))
                        && matches!(self.peek_at(1), Tok::Punct(":"))
                    {
                        let n = self.ident()?;
                        self.bump();
                        Some(n)
                    } else {
                        None
                    };
                    args.push(ElemArg::Expr(Arg {
                        name,
                        value: self.expr()?,
                    }));
                }
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
        }
        let children = if self.is("{") && !self.token().nl {
            self.bump();
            self.views()?
        } else {
            Vec::new()
        };
        Ok(Element {
            span: name.span.to(self.prev_span()),
            name,
            args,
            children,
        })
    }

    /// Raw tokens up to a top-level `,` or `)` (not consumed).
    fn raw_until_comma(&mut self) -> Result<(Vec<Token>, Span)> {
        let start = self.span();
        let mut toks = Vec::new();
        let mut depth = 0;
        loop {
            match self.peek() {
                Tok::Punct("," | ")") if depth == 0 => break,
                Tok::Punct("(") => depth += 1,
                Tok::Punct(")") => depth -= 1,
                Tok::Eof => return self.err("`)`"),
                _ => {}
            }
            toks.push(self.bump());
        }
        if toks.is_empty() {
            return self.err("a value");
        }
        Ok((toks, start.to(self.prev_span())))
    }

    /// Raw tokens of a declaration value: up to a newline, `;` or `}` at depth 0.
    fn raw_value(&mut self) -> Result<(Vec<Token>, Span)> {
        let start = self.span();
        let mut toks = Vec::new();
        let mut depth = 0;
        loop {
            let t = self.token();
            if depth == 0 && (!toks.is_empty() && t.nl || matches!(t.tok, Tok::Punct(";" | "}"))) {
                break;
            }
            match t.tok {
                Tok::Punct("(") => depth += 1,
                Tok::Punct(")") => depth -= 1,
                Tok::Eof => return self.err("`}`"),
                _ => {}
            }
            toks.push(self.bump());
        }
        self.eat(";");
        if toks.is_empty() {
            return self.err("a value");
        }
        Ok((toks, start.to(self.prev_span())))
    }

    /// A style property name; names may contain hyphens: `font-weight`.
    fn prop_name(&mut self) -> Result<Ident> {
        let mut prop = self.ident()?;
        while self.is("-") && self.span().lo == prop.span.hi {
            self.bump();
            let next = self.ident()?;
            prop.name = format!("{}-{}", prop.name, next.name);
            prop.span = prop.span.to(next.span);
        }
        Ok(prop)
    }

    fn style_block(&mut self) -> Result<StyleBlock> {
        let start = self.expect("{")?;
        let mut decls = Vec::new();
        let mut conds = Vec::new();
        while !self.eat("}") {
            if self.at_eof() {
                return Err(Diag::syntax(start, "unclosed style block"));
            }
            if self.is_kw("when") {
                let wspan = self.bump().span;
                let mut toks = Vec::new();
                while !self.is("{") {
                    if self.at_eof() {
                        return self.err("`{`");
                    }
                    toks.push(self.bump());
                }
                let span = wspan.to(self.prev_span());
                conds.push((toks, span, self.style_block()?));
                continue;
            }
            let prop = self.prop_name()?;
            self.expect(":")?;
            let (value, vspan) = self.raw_value()?;
            decls.push(StyleDecl {
                span: prop.span.to(vspan),
                prop,
                value,
            });
        }
        Ok(StyleBlock {
            decls,
            conds,
            span: start.to(self.prev_span()),
        })
    }
}

fn pat_binds(p: &Pattern, out: &mut Vec<String>) {
    match &p.kind {
        PatKind::Bind(n) => out.push(n.clone()),
        PatKind::Variant { args, .. } => args.iter().for_each(|a| pat_binds(a, out)),
        _ => {}
    }
}

fn keep_bind(p: &Pattern, name: &str) -> Pattern {
    let kind = match &p.kind {
        PatKind::Bind(n) if n != name => PatKind::Wild,
        PatKind::Variant {
            qual,
            name: vn,
            args,
        } => PatKind::Variant {
            qual: qual.clone(),
            name: vn.clone(),
            args: args.iter().map(|a| keep_bind(a, name)).collect(),
        },
        k => k.clone(),
    };
    Pattern { kind, span: p.span }
}
