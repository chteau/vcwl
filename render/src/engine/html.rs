//! Static HTML for prerendering: the element tree as markup that the browser styles with the
//! application's own style sheet, shown until the canvas takes over (`vcwl build` with
//! `prerender = true`). Search engines and browsers without JavaScript read it too.

use super::Engine;
use crate::dom::{Id, Kind};

/// The renderer's built-in styles that differ from the browser's (`style.rs` `defaults`).
pub const BASE_CSS: &str = "*,::before,::after{box-sizing:border-box}\
html,body{margin:0}html{overflow-x:hidden}\
body{font:16px/1.5 system-ui,-apple-system,'Segoe UI',sans-serif}\
h1,h2,h3,h4,h5,h6{line-height:1.5}\
code{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:.8125em}\
.v-col{display:flex;flex-direction:column}.v-row{display:flex}\
.v-stack{display:grid}.v-stack>*{grid-area:1/1}.v-grid{display:grid}\
.v-scroll{overflow:auto}.v-list{margin:0;padding-left:1.25em}\
.v-check{display:inline-flex;gap:.5em;align-items:center;cursor:pointer}\
.v-scene{display:block;height:300px;background:#111}";

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

/// A URL without a scheme, host or absolute path: `logo.svg`, `img/a.png`, `./x`.
fn is_relative(v: &str) -> bool {
    !v.is_empty()
        && !v.starts_with(['/', '?'])
        && !v
            .split(['/', '?', '#'])
            .next()
            .is_some_and(|first| first.contains(':'))
}

/// The URL of route `path` (`/docs/intro`) for path routing, from a page whose way to the site
/// root is `root` (`./`, `../../` or an absolute base such as `/repo/`): one directory per route.
pub fn route_url(root: &str, path: &str) -> String {
    let rel = path.trim_matches('/');
    if rel.is_empty() {
        root.to_string()
    } else {
        format!("{root}{rel}/")
    }
}

impl Engine {
    /// The tree below the root as HTML, and the style rules it needs beyond the application's
    /// sheet (bound styles become generated classes, so no inline styles are needed). With
    /// `root` (path routing, see [`route_url`]), route links `#/path` become real URLs.
    pub fn html(&self, root: Option<&str>) -> (String, String) {
        let (mut out, mut css) = (String::new(), String::new());
        for &c in &self.tree.n(0).children {
            self.node_html(c, root, &mut out, &mut css);
        }
        (out, css)
    }

    fn node_html(&self, id: Id, root: Option<&str>, out: &mut String, css: &mut String) {
        let n = self.tree.n(id);
        match n.kind {
            Kind::Comment => return,
            Kind::Text => return escape(out, &n.text, false),
            Kind::Element => {}
        }
        // A 3D scene is drawn by the host: the static page shows a placeholder of its size.
        let scene = n.tag == "scene";
        let tag = if is_name(&n.tag, "") && !scene {
            n.tag.as_str()
        } else {
            "div"
        };
        out.push('<');
        out.push_str(tag);
        let mut classes = n.classes.clone();
        if scene {
            classes.insert(0, "v-scene".into());
        }
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
            let link = match (k.as_str(), root, v.strip_prefix('#')) {
                ("href", Some(root), Some(path)) if path.starts_with('/') => route_url(root, path),
                // Relative to the site root, like the host resolves them, not to the route's page.
                ("href" | "src", Some(root), None) if is_relative(v) => format!("{root}{v}"),
                _ => v.to_string(),
            };
            out.push_str(&format!(" {k}=\""));
            escape(out, &link, true);
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
        } else if !scene {
            for &c in &n.children {
                self.node_html(c, root, out, css);
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
        let (html, css) = e.html(None);
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
        assert_eq!(route_url("../", "/docs/intro"), "../docs/intro/");
        assert_eq!(route_url("./", "/"), "./");
        let mut b = Buf::default();
        b.create(1, "a").attr(1, "href", "#/docs").insert(0, 1, 0);
        b.create(2, "a").attr(2, "href", "#top").insert(0, 2, 0);
        b.create(3, "img")
            .attr(3, "src", "logo.svg")
            .insert(0, 3, 0);
        b.create(4, "a")
            .attr(4, "href", "https://x.y/a")
            .insert(0, 4, 0);
        let mut e = Engine::new("", 800.0, 600.0);
        e.apply(&b.0).unwrap();
        let (html, _) = e.html(Some("/repo/"));
        assert!(
            html.contains("href=\"/repo/docs/\"") && html.contains("href=\"#top\""),
            "{html}"
        );
        assert!(html.contains("src=\"/repo/logo.svg\""), "{html}");
        assert!(html.contains("href=\"https://x.y/a\""), "{html}");
        assert!(e.html(None).0.contains("src=\"logo.svg\""));
        assert_eq!(safe_url("docs/a:b"), "docs/a:b");
        assert_eq!(safe_url("data:text/html,x"), "about:invalid#vcwl-blocked");
    }
}
