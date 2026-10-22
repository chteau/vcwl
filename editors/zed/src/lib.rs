//! Starts `vcwl lsp` for VCWL files. The `vcwl` binary is looked up on the PATH of the
//! worktree's shell (build it with `cargo install --path .` from the VCWL repository).

use zed_extension_api::{self as zed, LanguageServerId, Result};

struct Vcwl;

impl zed::Extension for Vcwl {
    fn new() -> Self {
        Vcwl
    }

    fn language_server_command(&mut self, _id: &LanguageServerId, worktree: &zed::Worktree) -> Result<zed::Command> {
        let command = worktree
            .which("vcwl")
            .ok_or("`vcwl` is not on your PATH (cargo install --path <vcwl repository>)")?;
        Ok(zed::Command { command, args: vec!["lsp".into()], env: Default::default() })
    }
}

zed::register_extension!(Vcwl);
