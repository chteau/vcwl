//! Detects Rust-isms in source that failed to parse and explains the VCWL spelling.

use crate::diag::{Diag, Span};

const MACROS: [&str; 5] = ["println", "format", "vec", "panic", "todo"];

pub fn diagnose(src: &str, base: u32) -> Option<Diag> {
    let b = src.as_bytes();
    let at = |lo: usize, hi: usize, msg: &str, help: &str| {
        Some(Diag::syntax(Span::new(base + lo as u32, base + hi as u32), msg).help(help))
    };
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < b.len() && !b[i..].starts_with(b"*/") {
                    i += 1;
                }
                i += 2;
            }
            b'\'' => {
                let mut j = i + 1;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                if j > i + 1 && b.get(j) != Some(&b'\'') {
                    return at(
                        i,
                        j,
                        "this is Rust. VCWL has no lifetimes; everything lives exactly as long as it is feeling useful",
                        "delete the lifetime annotation",
                    );
                }
                i = if b.get(j) == Some(&b'\'') { j + 1 } else { i + 1 };
            }
            b'#' if b.get(i + 1) == Some(&b'[') => {
                return at(
                    i,
                    i + 2,
                    "this is Rust. attributes are not a thing here; the compiler already knows what you meant, and it is not impressed",
                    "delete the attribute; structs and enums get equality and printing for free",
                );
            }
            b':' if b.get(i + 1) == Some(&b':') => {
                return at(
                    i,
                    i + 2,
                    "this is Rust. `::` is a path separator in a language that has more patience than we do",
                    "use `.` instead, e.g. `Color.Red`",
                );
            }
            b'&' => {
                if b.get(i + 1) == Some(&b'&') {
                    i += 2;
                    continue;
                }
                let mut hi = i + 1;
                if src[hi..].starts_with("mut ") {
                    hi += 3;
                }
                return at(
                    i,
                    hi,
                    "this is Rust. VCWL looks like Rust, the way a cardboard cutout looks like a person: there is no borrow checker here, only vibes",
                    "pass the value directly; drop the `&` and `&mut`",
                );
            }
            b'.' if src[i..].starts_with(".unwrap()") => {
                return at(
                    i,
                    i + 9,
                    "this is Rust. `.unwrap()` is how Rust programs say goodbye; VCWL would like you to say something first",
                    "handle the missing case with `match`",
                );
            }
            _ if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                let word = &src[start..i];
                let after = &src[i..];
                let prev_dot = start > 0 && b[start - 1] == b'.';
                if after.starts_with('!') && !after.starts_with("!=") && MACROS.contains(&word) {
                    let help = match word {
                        "println" => "use `print(...)`, e.g. `print(\"hello {name}\")`",
                        "format" => "use string interpolation: `\"hello {name}\"`",
                        "vec" => "arrays are written `[a, b, c]`",
                        _ => "VCWL has no macros; call a plain function",
                    };
                    return at(
                        start,
                        i + 1,
                        &format!("this is Rust. `{word}!` is a macro, and VCWL has none; it has functions and a sense of restraint"),
                        help,
                    );
                }
                if word == "let" && after.trim_start_matches(' ').starts_with("mut ") {
                    let off = after.len() - after.trim_start_matches(' ').len();
                    return at(
                        start,
                        i + off + 3,
                        "this is Rust. `let mut` is a bit much; VCWL just asks whether you plan to change your mind",
                        "use `var` for a mutable binding",
                    );
                }
                if (word == "Vec" || word == "Box") && after.starts_with('<') {
                    let help = if word == "Vec" {
                        "arrays are written `[T]`"
                    } else {
                        "there is no boxing; use the type directly"
                    };
                    return at(
                        start,
                        i + 1,
                        &format!("this is Rust. `{word}<..>` belongs to a language with an allocator it wants to tell you about"),
                        help,
                    );
                }
                if word == "pub" && after.starts_with(" fn") {
                    return at(
                        start,
                        i,
                        "this is Rust. `pub` is not a keyword here; visibility is a matter of `export`",
                        "write `export fn`",
                    );
                }
                if word == "use" && after.starts_with(' ') && !prev_dot {
                    return at(
                        start,
                        i,
                        "this is Rust. `use` imports are not how we do things here",
                        "write `import \"./file.vcwl\"` or `import \"@package\"`",
                    );
                }
                if word == "impl" && src[..start].trim_end().ends_with([':', '>']) {
                    return at(
                        start,
                        i,
                        "this is Rust. `impl Trait` in a signature is too clever by half",
                        "name an `interface` and use it as the type",
                    );
                }
            }
            _ => i += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(src: &str) -> Option<String> {
        diagnose(src, 0).map(|d| src[d.span.lo as usize..d.span.hi as usize].to_string())
    }

    #[test]
    fn detects() {
        assert_eq!(hit("fn main() { println!(\"x\") }").as_deref(), Some("println!"));
        assert_eq!(hit("let a = Foo::new()").as_deref(), Some("::"));
        assert_eq!(hit("let mut x = 1").as_deref(), Some("let mut"));
        assert_eq!(hit("fn f(a: &str) {}").as_deref(), Some("&"));
        assert_eq!(hit("fn f(a: &mut Foo) {}").as_deref(), Some("&mut"));
        assert_eq!(hit("#[derive(Debug)]").as_deref(), Some("#["));
        assert_eq!(hit("x.unwrap()").as_deref(), Some(".unwrap()"));
        assert_eq!(hit("let v: Vec<Int>").as_deref(), Some("Vec<"));
        assert_eq!(hit("pub fn f() {}").as_deref(), Some("pub"));
        assert_eq!(hit("use std.io").as_deref(), Some("use"));
        assert_eq!(hit("fn f<'a>() {}").as_deref(), Some("'a"));
        assert_eq!(hit("fn f(x: impl Show) {}").as_deref(), Some("impl"));
    }

    #[test]
    fn ignores_valid() {
        assert!(hit("let a = x && y // println!(1) ::").is_none());
        assert!(hit("print(\"a::b &x println!\") /* let mut */").is_none());
        assert!(hit("let c = 'a'\nif a != b {}").is_none());
    }
}
