//! Formatter tests: indentation is reconstructed from nesting alone, tokens never change.

use vcwl::fmt::format;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn dedent(src: &str) -> String {
    src.lines()
        .map(|l| format!("{}  \n", l.trim_start()))
        .collect()
}

#[test]
fn restores_indentation_of_repository_sources() {
    for f in [
        "src/prelude.vcwl",
        "examples/todo/src/main.vcwl",
        "examples/todo/src/components/todo_item.vcwl",
        "tests/programs/features.vcwl",
    ] {
        let src = std::fs::read_to_string(format!("{ROOT}/{f}")).unwrap();
        assert_eq!(format(f, &src).unwrap(), src, "{f} is not formatted");
        assert_eq!(
            format(f, &dedent(&src)).unwrap(),
            src,
            "{f}: indentation not restored"
        );
    }
}

#[test]
fn continuation_comments_and_blank_lines() {
    let src = "\n\n// lead\nfn f(xs: [Int]) -> Int {\nlet total = xs.len() +\n1\n\n\n/* block\n   kept as is */\nlet m = xs\n.map(|x| x * 2)\n.len()\n  // before close\n}\n\n";
    let want = "// lead\nfn f(xs: [Int]) -> Int {\n    let total = xs.len() +\n        1\n\n    /* block\n   kept as is */\n    let m = xs\n        .map(|x| x * 2)\n        .len()\n    // before close\n}\n";
    assert_eq!(format("t.vcwl", src).unwrap(), want);
    assert_eq!(format("t.vcwl", want).unwrap(), want);
}

#[test]
fn multi_line_strings_are_verbatim() {
    let src = "fn f() -> String {\n\"a\n  b {1 +\n2}\n\"\n}\n";
    assert_eq!(
        format("t.vcwl", src).unwrap(),
        "fn f() -> String {\n    \"a\n  b {1 +\n2}\n\"\n}\n"
    );
}

#[test]
fn reports_lex_errors_with_location() {
    let e = format("bad.vcwl", "fn f() {\n    let s = 1 $ 2\n}\n").unwrap_err();
    assert!(e.contains("bad.vcwl:2:"), "{e}");
}

#[test]
fn keeps_crlf_line_endings() {
    let src = "component App {\r\ntext(\"hi\")\r\n}\r\n";
    assert_eq!(
        format("a.vcwl", src).unwrap(),
        "component App {\r\n    text(\"hi\")\r\n}\r\n"
    );
}
