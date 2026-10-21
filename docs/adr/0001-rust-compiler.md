# 1. Write the compiler in Rust

Status: accepted

## Context

The toolchain must be a single fast, portable binary (compiler, dev server, formatter, test
runner), with mature libraries to emit and validate WebAssembly.

## Decision

Rust, edition 2024, with two dependencies: `wasm-encoder` (binary emission) and `wasmparser`
(independent validation of every module we emit). Both are maintained by the Bytecode Alliance as
part of `wasm-tools`. The HTTP dev server, file watching and formatting use only the standard
library.

## Consequences

Validation catches code generation bugs at compile time instead of in the browser. The small
dependency set keeps builds fast; features such as file watching use polling instead of
platform-specific crates.
