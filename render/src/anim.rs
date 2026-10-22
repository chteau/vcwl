//! CSS transitions of the properties that only affect painting: opacity, transform, color and
//! background color. Layout properties change immediately.

use crate::css::Tok;
use crate::style::{Background, Color, Len, Style, Transform};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Prop {
    All,
    Opacity,
    Transform,
    Color,
    Background,
}

const PROPS: [Prop; 4] = [
    Prop::Opacity,
    Prop::Transform,
    Prop::Color,
    Prop::Background,
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transition {
    pub prop: Prop,
    /// Duration and delay in milliseconds.
    pub dur: f32,
    pub delay: f32,
    /// Cubic Bézier control points (x1, y1, x2, y2).
    pub ease: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Num(f32),
    Color(Color),
    Transform(Vec<Transform>),
}

#[derive(Clone, Debug)]
pub struct Anim {
    pub prop: Prop,
    pub from: Value,
    pub to: Value,
    pub start: f64,
    pub t: Transition,
}

/// Parses a `transition` value; properties that cannot be animated are ignored.
pub fn parse(v: &[Tok]) -> Vec<Transition> {
    v.split(|t| *t == Tok::Comma)
        .filter_map(|group| {
            let mut t = Transition {
                prop: Prop::All,
                dur: 0.0,
                delay: 0.0,
                ease: [0.25, 0.1, 0.25, 1.0],
            };
            let mut times = 0;
            for tok in group {
                match tok {
                    Tok::Num(n, u) if u == "ms" || u == "s" => {
                        let ms = if u == "s" { n * 1000.0 } else { *n };
                        if times == 0 {
                            t.dur = ms
                        } else {
                            t.delay = ms
                        }
                        times += 1;
                    }
                    Tok::Ident(k) => match k.as_str() {
                        "all" => t.prop = Prop::All,
                        "opacity" => t.prop = Prop::Opacity,
                        "transform" => t.prop = Prop::Transform,
                        "color" => t.prop = Prop::Color,
                        "background" | "background-color" => t.prop = Prop::Background,
                        "linear" => t.ease = [0.0, 0.0, 1.0, 1.0],
                        "ease" => {}
                        "ease-in" => t.ease = [0.42, 0.0, 1.0, 1.0],
                        "ease-out" => t.ease = [0.0, 0.0, 0.58, 1.0],
                        "ease-in-out" => t.ease = [0.42, 0.0, 0.58, 1.0],
                        // Not CSS: marked by a negative first control point.
                        "spring" => t.ease = [SPRING, 0.0, 0.0, 0.0],
                        "bounce" => t.ease = [BOUNCE, 0.0, 0.0, 0.0],
                        _ => return None,
                    },
                    _ => return None,
                }
            }
            (t.dur > 0.0).then_some(t)
        })
        .collect()
}

fn get(s: &Style, p: Prop) -> Option<Value> {
    Some(match p {
        Prop::Opacity => Value::Num(s.opacity),
        Prop::Transform => Value::Transform(s.transform.clone()),
        Prop::Color => Value::Color(s.color),
        Prop::Background => match s.background {
            Background::None => Value::Color(0),
            Background::Color(c) => Value::Color(c),
            Background::Linear(..) => return None,
        },
        Prop::All => return None,
    })
}

fn set(s: &mut Style, p: Prop, v: Value) {
    match (p, v) {
        (Prop::Opacity, Value::Num(n)) => s.opacity = n,
        (Prop::Transform, Value::Transform(t)) => s.transform = t,
        (Prop::Color, Value::Color(c)) => s.color = c,
        (Prop::Background, Value::Color(c)) => s.background = Background::Color(c),
        _ => {}
    }
}

/// Starts, retargets or cancels the animations of a node whose style went from `old` to `new`.
pub fn start(old: &Style, new: &Style, running: &mut Vec<Anim>, now: f64) {
    for p in PROPS {
        let current = running
            .iter()
            .find(|a| a.prop == p)
            .map(|a| sample(a, now).0);
        running.retain(|a| a.prop != p);
        let Some(t) = new
            .transitions
            .iter()
            .rev()
            .find(|t| t.prop == p || t.prop == Prop::All)
        else {
            continue;
        };
        let (Some(from), Some(to)) = (current.or_else(|| get(old, p)), get(new, p)) else {
            continue;
        };
        if from != to {
            running.push(Anim {
                prop: p,
                from,
                to,
                start: now,
                t: *t,
            });
        }
    }
}

const SPRING: f32 = -1.0;
const BOUNCE: f32 = -2.0;

/// Progress at `x` (0 to 1) for an easing: a cubic Bézier, a damped spring that overshoots
/// and settles, or a ball bouncing to rest.
fn ease(e: [f32; 4], x: f32) -> f32 {
    if e[0] == SPRING {
        return 1.0 - (-6.0 * x).exp() * (12.0 * x).cos();
    }
    if e[0] == BOUNCE {
        let (n, d) = (7.5625, 2.75);
        return if x < 1.0 / d {
            n * x * x
        } else if x < 2.0 / d {
            let x = x - 1.5 / d;
            n * x * x + 0.75
        } else if x < 2.5 / d {
            let x = x - 2.25 / d;
            n * x * x + 0.9375
        } else {
            let x = x - 2.625 / d;
            n * x * x + 0.984375
        };
    }
    bezier(e, x)
}

/// Value of `a` at `now`, and whether it has finished.
pub fn sample(a: &Anim, now: f64) -> (Value, bool) {
    let x = ((now - a.start) as f32 - a.t.delay) / a.t.dur;
    if x >= 1.0 {
        return (a.to.clone(), true);
    }
    let k = ease(a.t.ease, x.max(0.0));
    (lerp(&a.from, &a.to, k), false)
}

/// Writes the current animated values into `s`; returns false once all have finished (they
/// are then removed).
pub fn apply(s: &mut Style, running: &mut Vec<Anim>, now: f64) -> bool {
    running.retain(|a| {
        let (v, done) = sample(a, now);
        set(s, a.prop, v);
        !done
    });
    !running.is_empty()
}

fn bezier(e: [f32; 4], x: f32) -> f32 {
    if e[0] == e[1] && e[2] == e[3] {
        return x;
    }
    let f = |a: f32, b: f32, t: f32| {
        let u = 1.0 - t;
        3.0 * a * t * u * u + 3.0 * b * t * t * u + t * t * t
    };
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..20 {
        let m = (lo + hi) / 2.0;
        if f(e[0], e[2], m) < x { lo = m } else { hi = m }
    }
    f(e[1], e[3], (lo + hi) / 2.0)
}

fn mix(a: f32, b: f32, k: f32) -> f32 {
    a + (b - a) * k
}

fn mix_color(a: Color, b: Color, k: f32) -> Color {
    // Fading from or to transparent keeps the other color's hue.
    let (a, b) = match (a & 0xff, b & 0xff) {
        (0, _) => (b & !0xff, b),
        (_, 0) => (a, a & !0xff),
        _ => (a, b),
    };
    (0..4).fold(0, |c, i| {
        let sh = 24 - i * 8;
        let v = mix(((a >> sh) & 0xff) as f32, ((b >> sh) & 0xff) as f32, k);
        c | ((v + 0.5) as u32).min(255) << sh
    })
}

fn identity(t: &Transform) -> Transform {
    match t {
        Transform::Translate(..) => Transform::Translate(Len::Px(0.0), Len::Px(0.0)),
        Transform::Scale(..) => Transform::Scale(1.0, 1.0),
        Transform::Rotate(_) => Transform::Rotate(0.0),
    }
}

fn mix_len(a: Len, b: Len, k: f32) -> Option<Len> {
    match (a, b) {
        (Len::Px(x), Len::Px(y)) => Some(Len::Px(mix(x, y, k))),
        (Len::Pct(x), Len::Pct(y)) => Some(Len::Pct(mix(x, y, k))),
        _ => None,
    }
}

fn lerp(a: &Value, b: &Value, k: f32) -> Value {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => Value::Num(mix(*x, *y, k)),
        (Value::Color(x), Value::Color(y)) => Value::Color(mix_color(*x, *y, k)),
        (Value::Transform(x), Value::Transform(y)) => {
            let n = x.len().max(y.len());
            let pick = |v: &[Transform], o: &[Transform], i: usize| {
                v.get(i).cloned().unwrap_or_else(|| identity(&o[i]))
            };
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let (p, q) = (pick(x, y, i), pick(y, x, i));
                out.push(match (p, q) {
                    (Transform::Translate(x0, y0), Transform::Translate(x1, y1)) => {
                        match (mix_len(x0, x1, k), mix_len(y0, y1, k)) {
                            (Some(x), Some(y)) => Transform::Translate(x, y),
                            _ => return if k < 0.5 { a.clone() } else { b.clone() },
                        }
                    }
                    (Transform::Scale(x0, y0), Transform::Scale(x1, y1)) => {
                        Transform::Scale(mix(x0, x1, k), mix(y0, y1, k))
                    }
                    (Transform::Rotate(r0), Transform::Rotate(r1)) => {
                        Transform::Rotate(mix(r0, r1, k))
                    }
                    _ => return if k < 0.5 { a.clone() } else { b.clone() },
                });
            }
            Value::Transform(out)
        }
        _ => {
            if k < 0.5 {
                a.clone()
            } else {
                b.clone()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_interpolate() {
        let t = parse(&crate::css::tokens(
            "opacity 200ms linear, width 1s, transform .5s ease-out 100ms",
        ));
        assert_eq!(t.len(), 2);
        assert_eq!((t[1].dur, t[1].delay), (500.0, 100.0));
        let old = Style {
            opacity: 0.0,
            transform: vec![Transform::Translate(Len::Px(0.0), Len::Px(20.0))],
            ..Style::default()
        };
        let new = Style {
            transitions: t,
            ..Style::default()
        };
        let mut run = Vec::new();
        start(&old, &new, &mut run, 0.0);
        assert_eq!(run.len(), 2);
        let mut s = new.clone();
        assert!(apply(&mut s, &mut run, 100.0));
        assert!((s.opacity - 0.5).abs() < 1e-3);
        assert_eq!(s.transform, old.transform, "transform waits for its delay");
        // Retargeting starts from the current value.
        let back = Style {
            opacity: 0.0,
            transitions: new.transitions.clone(),
            ..Style::default()
        };
        start(&new, &back, &mut run, 100.0);
        assert_eq!(
            run.iter().find(|a| a.prop == Prop::Opacity).unwrap().from,
            Value::Num(0.5)
        );
        assert!(!apply(&mut s, &mut run, 10_000.0));
        assert!(run.is_empty());
        assert_eq!(mix_color(0x00000000, 0xff000080, 0.5), 0xff000040);
    }

    #[test]
    fn spring_overshoots_and_bounce_lands() {
        let t = parse(&crate::css::tokens(
            "transform 600ms spring, opacity 1s bounce",
        ));
        let peak = (1..100)
            .map(|i| ease(t[0].ease, i as f32 / 100.0))
            .fold(0.0, f32::max);
        assert!(peak > 1.05, "a spring goes past its target: {peak}");
        assert!((ease(t[0].ease, 1.0) - 1.0).abs() < 0.01);
        let b = |x| ease(t[1].ease, x);
        assert!(
            b(0.5) < 1.0 && (b(1.0) - 1.0).abs() < 1e-4 && b(0.36) > b(0.45),
            "it bounces"
        );
    }
}
