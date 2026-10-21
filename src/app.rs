//! Web application compilation: components, styles, capabilities and artifacts.

use crate::check::Checker;
use crate::compile::Options;
use crate::diag::Result;
use crate::lower::Lower;

pub struct AppInfo;

pub struct AppOutput {
    pub title: String,
    pub css: String,
    pub capabilities: String,
}

pub fn check_app(_ck: &mut Checker) -> Option<AppInfo> {
    None
}

pub fn lower_app(_lw: &mut Lower, _ck: &Checker, _app: AppInfo, _opts: &Options) -> Result<AppOutput> {
    unreachable!()
}
