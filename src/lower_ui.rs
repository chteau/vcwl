//! Lowering of components to DOM-building and update functions.

use crate::ir::IExpr;
use crate::lower::{FnCx, Lower};
use crate::tir::{TExpr, TFetch, Type};

pub struct UiState;
pub type ScopeRef = usize;

impl Lower<'_> {
    pub fn ui_read(&mut self, _cx: &mut FnCx, _e: &TExpr) -> IExpr {
        unreachable!("component values outside a component")
    }
    pub fn ui_assign(&mut self, _cx: &mut FnCx, _var: usize, _e: &TExpr) -> IExpr {
        unreachable!("state assignment outside a component")
    }
    pub fn ui_intrinsic(&mut self, name: &str, _targs: &[Type], _args: Vec<IExpr>) -> IExpr {
        unreachable!("unknown intrinsic {name}")
    }
    pub fn lower_fetch(&mut self, _cx: &mut FnCx, _f: &TFetch) -> IExpr {
        unreachable!("fetch outside a handler")
    }
}
