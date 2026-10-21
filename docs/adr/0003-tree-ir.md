# 3. Two IRs: typed tree (TIR) and a WebAssembly-shaped tree (IR)

Status: accepted

## Context

The checker needs a representation with types and generics; the code generator needs explicit
locals, labels and value types.

## Decision

The checker produces TIR (types on every node, generics unresolved). Lowering monomorphizes and
produces IR, a tree whose nodes map almost one-to-one to WebAssembly instructions with structured
control flow. Optimizations (constant folding, tree shaking) run on IR.

## Consequences

Code generation is a direct walk; no SSA or register allocation. Optimization opportunities that
need data-flow analysis are left to the engine's optimizing compiler.
