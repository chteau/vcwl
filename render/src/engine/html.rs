//! Static HTML for prerendering: the element tree as markup that the browser styles with the
//! application's own style sheet, shown until the canvas takes over (`vcwl build` with
//! `prerender = true`). Search engines and browsers without JavaScript read it too.

use super::Engine;
use crate::dom::{Id, Kind};

/// The renderer's built-in styles that differ from the browser's (`style.rs` `defaults`).
pub const BASE_CSS: &str = "*,::before,::after{box-sizing:border-box}\
html,body{margin:0}\
body{font:16px/1.5 system-ui,-apple-system,'Segoe UI',sans-serif}\
h1,h2,h3,h4,h5,h6{line-height:1.5}\
code{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:.8125em}\
.v-col{display:flex;flex-direction:column}.v-row{display:flex}\
.v-stack{display:grid}.v-stack>*{grid-area:1/1}.v-grid{display:grid}\
.v-scroll{overflow:auto}.v-list{margin:0;padding-left:1.25em}\
.v-check{display:inline-flex;gap:.5em;align-items:center;cursor:pointer}";

/// Attributes copied besides `aria-*`.
const ATTRS: &[&str] = &[
    "id",
    "href",
    "src",
    "alt",
    "title",
    "role",
    "lang",
    "type",
    "placeholder",
    "disabled",
    "target",
    "rel",
];

const VOID: &[&str] = &["input", "img", "hr", "br"];

fn escape(out: &mut String, s: &str, attr: bool) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

/// Rejects `javascript:` and other script-capable URLs (as `sanitizeUrl` in the host does):
/// http(s), mailto and tel URLs and relative references pass.
pub fn safe_url(v: &str) -> &str {
    let v = v.trim();
    let scheme = v
        .find(|c: char| !(c.is_ascii_alphanumeric() || "+.-".contains(c)))
        .filter(|&i| v[i..].starts_with(':') && v.starts_with(|c: char| c.is_ascii_alphabetic()))
        .map(|i| v[..i].to_ascii_lowercase());
    match scheme.as_deref() {
        None | Some("http" | "https" | "mailto" | "tel") => v,
        _ => "about:invalid#vcwl-blocked",
    }
}

fn is_name(s: &str, extra: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || extra.contains(c))
}

impl Engine {
    /// The tree below the root as HTML, and the style rules it needs beyond the application's
    /// sheet (bound styles become generated classes, so no inline styles are needed).
    pub fn html(&self) -> (String, String) {
        let (mut out, mut css) = (String::new(), String::new());
        for &c in &self.tree.n(0).children {
            self.node_html(c, &mut out, &mut css);
        }
        (out, css)
    }

    fn node_html(&self, id: Id, out: &mut String, css: &mut String) {
        let n = self.tree.n(id);
        match n.kind {
            Kind::Comment => return,
            Kind::Text => return escape(out, &n.text, false),
            Kind::Element => {}
        }
        let tag = if is_name(&n.tag, "") {
            n.tag.as_str()
        } else {
            "div"
        };
        out.push('<');
        out.push_str(tag);
        let mut classes = n.classes.clone();
        let mut bound = String::new();
        for (k, v) in &n.attrs {
            if let Some((prop, unit)) = k.strip_prefix("style:").and_then(|r| r.split_once(':')) {
                let number = v.parse::<f64>().is_ok_and(f64::is_finite);
                if is_name(prop, "-") && is_name(unit, "%") && number {
                    bound.push_str(&format!("{prop}:{v}{unit};"));
                }
                continue;
            }
            if !(k.starts_with("aria-") && is_name(k, "-") || ATTRS.contains(&k.as_str())) {
                continue;
            }
            let v = if k == "href" || k == "src" {
                safe_url(v)
            } else {
                v
            };
            out.push_str(&format!(" {k}=\""));
            escape(out, v, true);
            out.push('"');
        }
        if !bound.is_empty() {
            // FNV-1a names the rule, so equal styles share a class across pages.
            let h = bound.bytes().fold(0x811c9dc5u32, |h, b| {
                (h ^ b as u32).wrapping_mul(0x01000193)
            });
            let class = format!("vb{h:08x}");
            let rule = format!(".{class}{{{bound}}}");
            if !css.contains(&rule) {
                css.push_str(&rule);
            }
            classes.push(class);
        }
        if !classes.is_empty() {
            out.push_str(" class=\"");
            escape(out, &classes.join(" "), true);
            out.push('"');
        }
        match tag {
            "input" if n.attr("type") == Some("checkbox") => {
                if n.checked {
                    out.push_str(" checked");
                }
            }
            "input" => {
                out.push_str(" value=\"");
                escape(out, &n.value, true);
                out.push('"');
            }
            _ => {}
        }
        out.push('>');
        if VOID.contains(&tag) {
            return;
        }
        if tag == "textarea" {
            escape(out, &n.value, false);
        } else {
            for &c in &n.children {
                self.node_html(c, out, css);
            }
        }
        out.push_str("</");
        out.push_str(tag);
        out.push('>');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::tests::Buf;

    #[test]
    fn serializes_safe_markup() {
        let mut b = Buf::default();
        b.create(1, "div").class(1, "v-col a").insert(0, 1, 0);
        b.create(2, "a")
            .attr(2, "href", "javascript:alert(1)")
            .attr(2, "data-x", "1")
            .insert(1, 2, 0)
            .text(3, "<b>&\"")
            .insert(2, 3, 0);
        b.create(4, "div")
            .attr(4, "style:width:%", "42.5")
            .attr(4, "style:height:px", "1;color:red")
            .insert(1, 4, 0);
        b.create(5, "input")
            .attr(5, "aria-label", "Q\"")
            .insert(1, 5, 0);
        let mut e = Engine::new("", 800.0, 600.0);
        e.apply(&b.0).unwrap();
        let (html, css) = e.html();
        assert!(
            !html.contains("javascript") && !html.contains("data-x"),
            "{html}"
        );
        assert!(html.contains("&lt;b&gt;&amp;\""), "{html}");
        assert!(html.contains("aria-label=\"Q&quot;\""), "{html}");
        assert!(!css.contains("color"), "unsafe bound value dropped: {css}");
        assert!(css.contains("{width:42.5%;}"), "{css}");
        assert_eq!(safe_url(" HTTPS://x.y/"), "HTTPS://x.y/");
        assert_eq!(safe_url("#/docs"), "#/docs");
        assert_eq!(safe_url("docs/a:b"), "docs/a:b");
        assert_eq!(safe_url("data:text/html,x"), "about:invalid#vcwl-blocked");
    }
}
