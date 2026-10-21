//! Checking of components and views.

use crate::check::Checker;
use crate::diag::{Diag, Result, Span};
use crate::tir::*;

#[derive(Default)]
pub struct UiEnv {
    pub names: Vec<String>,
}

impl UiEnv {
    pub fn names(&self) -> Vec<String> {
        self.names.clone()
    }
}

impl Checker<'_> {
    pub fn ui_ident(&mut self, _name: &str, _span: Span) -> Result<Option<TExpr>> {
        Ok(None)
    }

    pub fn ui_place(&mut self, name: &str, span: Span) -> Result<(Place, Type)> {
        Err(Diag::ty(span, format!("unknown variable `{name}`")))
    }

    pub fn check_fetch(&mut self, f: &crate::ast::Fetch) -> Result<TStmt> {
        Err(Diag::ty(f.span, "`fetch` can only be used inside an event handler"))
    }
}
