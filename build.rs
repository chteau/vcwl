//! Builds the renderer (`render/`) for WebAssembly so the CLI can embed it in apps.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=render/src");
    println!("cargo:rerun-if-changed=render/Cargo.toml");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let target_dir = out.join("render-target");
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .args([
            "rustc",
            "-q",
            "-p",
            "vcwl-render",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
            "--crate-type",
            "cdylib",
        ])
        .args([
            "--config",
            "profile.release.lto=true",
            "--config",
            "profile.release.opt-level=\"s\"",
        ])
        .args([
            "--config",
            "profile.release.codegen-units=1",
            "--config",
            "profile.release.panic=\"abort\"",
        ])
        .args([
            "--config",
            "target.wasm32-unknown-unknown.rustflags=[\"-C\",\"link-arg=--strip-all\"]",
        ])
        .env("CARGO_TARGET_DIR", &target_dir)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET")
        .status()
        .expect("failed to run cargo for the renderer");
    if !status.success() {
        panic!(
            "building the renderer failed; the wasm target may be missing: rustup target add wasm32-unknown-unknown"
        );
    }
    let wasm = target_dir.join("wasm32-unknown-unknown/release/vcwl_render.wasm");
    std::fs::copy(&wasm, out.join("render.wasm")).expect("renderer output missing");
}
