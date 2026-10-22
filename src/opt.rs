//! IR optimizations: constant folding and removal of unreachable functions and imports.

use crate::ir::*;

pub fn optimize(m: &mut Module) {
    for f in &mut m.funcs {
        f.body.iter_mut().for_each(fold);
    }
    tree_shake(m);
}

fn fold(e: &mut IExpr) {
    visit_children(e, fold);
    let folded = match e {
        IExpr::Bin(op, a, b) => match (&**a, &**b) {
            (IExpr::I64(x), IExpr::I64(y)) => fold_i64(*op, *x, *y),
            (IExpr::I32(x), IExpr::I32(y)) => fold_i32(*op, *x, *y),
            (IExpr::F64(x), IExpr::F64(y)) => fold_f64(*op, *x, *y),
            _ => None,
        },
        IExpr::Un(UnOp::I32Eqz, a) => match **a {
            IExpr::I32(x) => Some(IExpr::I32((x == 0) as i32)),
            _ => None,
        },
        IExpr::If(_, c, t, f) => match **c {
            IExpr::I32(x) => Some(IExpr::Seq(if x != 0 {
                std::mem::take(t)
            } else {
                std::mem::take(f)
            })),
            _ => None,
        },
        _ => None,
    };
    if let Some(n) = folded {
        *e = n;
    }
}

fn fold_i64(op: Op, x: i64, y: i64) -> Option<IExpr> {
    use Op::*;
    Some(match op {
        I64Add => IExpr::I64(x.wrapping_add(y)),
        I64Sub => IExpr::I64(x.wrapping_sub(y)),
        I64Mul => IExpr::I64(x.wrapping_mul(y)),
        // Division keeps its runtime trap semantics for zero and overflow.
        I64DivS if y != 0 && !(x == i64::MIN && y == -1) => IExpr::I64(x / y),
        I64RemS if y != 0 && !(x == i64::MIN && y == -1) => IExpr::I64(x % y),
        I64Eq => IExpr::I32((x == y) as i32),
        I64Ne => IExpr::I32((x != y) as i32),
        I64LtS => IExpr::I32((x < y) as i32),
        I64LeS => IExpr::I32((x <= y) as i32),
        I64GtS => IExpr::I32((x > y) as i32),
        I64GeS => IExpr::I32((x >= y) as i32),
        _ => return None,
    })
}

fn fold_i32(op: Op, x: i32, y: i32) -> Option<IExpr> {
    use Op::*;
    Some(match op {
        I32Add => IExpr::I32(x.wrapping_add(y)),
        I32Sub => IExpr::I32(x.wrapping_sub(y)),
        I32Mul => IExpr::I32(x.wrapping_mul(y)),
        I32And => IExpr::I32(x & y),
        I32Or => IExpr::I32(x | y),
        I32Eq => IExpr::I32((x == y) as i32),
        I32Ne => IExpr::I32((x != y) as i32),
        _ => return None,
    })
}

fn fold_f64(op: Op, x: f64, y: f64) -> Option<IExpr> {
    use Op::*;
    Some(match op {
        F64Add => IExpr::F64(x + y),
        F64Sub => IExpr::F64(x - y),
        F64Mul => IExpr::F64(x * y),
        F64Div => IExpr::F64(x / y),
        F64Lt => IExpr::I32((x < y) as i32),
        F64Gt => IExpr::I32((x > y) as i32),
        _ => return None,
    })
}

pub fn visit_children(e: &mut IExpr, mut f: impl FnMut(&mut IExpr)) {
    use IExpr::*;
    match e {
        LocalSet(_, a)
        | LocalTee(_, a)
        | GlobalSet(_, a)
        | Un(_, a)
        | Drop(a)
        | StructGet(_, _, a)
        | ArrayNewDefault(_, a)
        | ArrayLen(a)
        | RefCast(_, a)
        | RefTest(_, a)
        | Load8U(a)
        | Load32(a)
        | MemoryGrow(a)
        | BrIf(_, a)
        | MemoryInit(a, _, _) => f(a),
        Bin(_, a, b2)
        | StructSet(_, _, a, b2)
        | ArrayNew(_, a, b2)
        | ArrayGet(_, a, b2)
        | ArrayGetU(_, a, b2)
        | RefEq(a, b2)
        | Store8(a, b2)
        | Store32(a, b2) => {
            f(a);
            f(b2);
        }
        ArraySet(_, a, b2, c) => {
            f(a);
            f(b2);
            f(c);
        }
        ArrayCopy(_, xs) => xs.iter_mut().for_each(|x| f(x)),
        Call(_, xs)
        | Seq(xs)
        | StructNew(_, xs)
        | ArrayNewFixed(_, xs)
        | Block(_, _, xs)
        | Loop(_, xs) => xs.iter_mut().for_each(f),
        CallRef(_, xs, g) => {
            xs.iter_mut().for_each(&mut f);
            f(g);
        }
        If(_, c, t, el) => {
            f(c);
            t.iter_mut().for_each(&mut f);
            el.iter_mut().for_each(f);
        }
        Return(Some(a)) => f(a),
        I32(_) | I64(_) | F64(_) | Null(_) | NullAny | LocalGet(_) | GlobalGet(_) | Br(_)
        | Return(None) | Unreachable | ArrayNewData(..) | MemorySize | StructNewDefault(_)
        | RefFunc(_) => {}
    }
}

fn calls(e: &mut IExpr, out: &mut Vec<FuncRef>) {
    match e {
        IExpr::Call(f, _) => out.push(*f),
        IExpr::RefFunc(f) => out.push(FuncRef::Local(*f)),
        _ => {}
    }
    visit_children(e, |c| calls(c, out));
}

/// Keeps only functions reachable from exports, and only the imports they use.
fn tree_shake(m: &mut Module) {
    let n = m.funcs.len();
    let mut live = vec![false; n];
    let mut live_imports = vec![false; m.imports.len()];
    let mut stack: Vec<usize> = (0..n).filter(|&i| m.funcs[i].export.is_some()).collect();
    for &i in &stack {
        live[i] = true;
    }
    while let Some(i) = stack.pop() {
        let mut refs = Vec::new();
        for e in &mut m.funcs[i].body {
            calls(e, &mut refs);
        }
        for r in refs {
            match r {
                FuncRef::Local(j) if !live[j as usize] => {
                    live[j as usize] = true;
                    stack.push(j as usize);
                }
                FuncRef::Import(j) => live_imports[j as usize] = true,
                _ => {}
            }
        }
    }
    let remap = |flags: &[bool]| {
        let mut next = 0u32;
        flags
            .iter()
            .map(|&l| {
                if l {
                    next += 1;
                    Some(next - 1)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    let fmap = remap(&live);
    let imap = remap(&live_imports);
    fn rewrite(e: &mut IExpr, fmap: &[Option<u32>], imap: &[Option<u32>]) {
        match e {
            IExpr::Call(f, _) => {
                *f = match *f {
                    FuncRef::Local(j) => FuncRef::Local(fmap[j as usize].unwrap()),
                    FuncRef::Import(j) => FuncRef::Import(imap[j as usize].unwrap()),
                };
            }
            IExpr::RefFunc(f) => *f = fmap[*f as usize].unwrap(),
            _ => {}
        }
        visit_children(e, |c| rewrite(c, fmap, imap));
    }
    m.sig_override = m
        .sig_override
        .iter()
        .filter_map(|&(f, t)| fmap[f as usize].map(|nf| (nf, t)))
        .collect();
    let funcs = std::mem::take(&mut m.funcs);
    m.funcs = funcs
        .into_iter()
        .zip(&live)
        .filter(|(_, l)| **l)
        .map(|(f, _)| f)
        .collect();
    for f in &mut m.funcs {
        f.body.iter_mut().for_each(|e| rewrite(e, &fmap, &imap));
    }
    let imports = std::mem::take(&mut m.imports);
    m.imports = imports
        .into_iter()
        .zip(&live_imports)
        .filter(|(_, l)| **l)
        .map(|(f, _)| f)
        .collect();
}
