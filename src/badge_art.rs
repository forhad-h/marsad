//! Badge art: a deterministic Islamic geometric star per tier, as strokes in a unit space and as an SVG fragment.
// used by the card (U4) and the badge view (U6)
#![allow(dead_code)]

use std::f64::consts::PI;
use std::fmt::Write;

/// A point in the unit badge space: centre at the origin, y up, the badge rim at radius 1.
pub type Point = (f64, f64);

/// The number of tiers, Novice (0) to Grandmaster (6).
pub const TIERS: usize = 7;

/// An sRGB colour.
pub type Rgb = (u8, u8, u8);

/// The open range of contact angles that give an n-point star: at the low end the valleys reach the tips, at the high end the centre.
pub fn contact_bounds(n: usize) -> (f64, f64) {
    (PI / (2.0 * n as f64), PI / 2.0)
}

/// A Hankin star in a regular n-gon whose edge midpoints lie at `radius`, the first at angle `rotation`.
/// From each edge midpoint two rays leave at `contact` to the edge; each ray meets the opposite ray of the neighbouring edge.
/// Returns the closed outline: midpoint (tip), meeting point (valley), next midpoint, and so on, 2n points.
pub fn hankin_star(n: usize, radius: f64, rotation: f64, contact: f64) -> Option<Vec<Point>> {
    let (lo, hi) = contact_bounds(n);
    // Written so NaN fails both checks.
    let in_bounds = contact > lo && contact < hi;
    let positive = radius > 0.0;
    if n < 3 || !in_bounds || !positive {
        return None;
    }
    let step = 2.0 * PI / n as f64;
    let mid = |i: usize| {
        let a = rotation + step * i as f64;
        (radius * a.cos(), radius * a.sin())
    };
    // The ray from midpoint i towards vertex i+1, turned inward by the contact angle.
    let ray = |i: usize, sign: f64| {
        let a = rotation + step * i as f64;
        let (tx, ty) = (-a.sin() * sign, a.cos() * sign);
        let (nx, ny) = (-a.cos(), -a.sin());
        (tx * contact.cos() + nx * contact.sin(), ty * contact.cos() + ny * contact.sin())
    };
    let mut out = Vec::with_capacity(2 * n);
    for i in 0..n {
        let (p, u) = (mid(i), ray(i, 1.0));
        let (q, v) = (mid(i + 1), ray(i + 1, -1.0));
        out.push(p);
        out.push(intersect(p, u, q, v)?);
    }
    Some(out)
}

/// Where the line p + s·u meets the line q + t·v, if they are not parallel.
fn intersect(p: Point, u: Point, q: Point, v: Point) -> Option<Point> {
    let den = u.0 * v.1 - u.1 * v.0;
    if den.abs() < 1e-12 {
        return None;
    }
    let s = ((q.0 - p.0) * v.1 - (q.1 - p.1) * v.0) / den;
    Some((p.0 + s * u.0, p.1 + s * u.1))
}

/// The {n/k} star: contact πk/n sends each ray to the tip k steps on.
fn star_contact(n: usize, k: usize) -> f64 {
    PI * k as f64 / n as f64
}

/// What a stroke is part of; the badge view colours by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Ring,
    Star,
    Rosette,
}

/// A polyline in the unit space.
#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub points: Vec<Point>,
    pub closed: bool,
    pub role: Role,
}

/// A filled polygon in the unit space.
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    pub points: Vec<Point>,
    pub color: Rgb,
}

/// A tier's colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// The disc behind the pattern.
    pub ground: Rgb,
    /// The main star's field.
    pub field: Rgb,
    /// The inner rosette's field.
    pub rosette: Rgb,
    /// The wide stroke under the strapwork.
    pub band: Rgb,
    /// The narrow stroke on top of the strapwork.
    pub line: Rgb,
}

/// How a tier's badge is built.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Recipe {
    /// The main star's point count.
    pub points: usize,
    /// The main star is {points/k}.
    pub k: usize,
    /// A second {points/k} star, turned half a step and laced through the first.
    pub double: Option<usize>,
    /// Draw the stars' full chords so the lines cross, not just their outlines; the chords stop at the first rosette.
    pub interlace: bool,
    /// Inner rosettes, outermost first: each one's radius and k, turned half a step from the one outside it.
    pub rosettes: &'static [(f64, usize)],
    /// Concentric rings around the star.
    pub rings: usize,
    pub palette: Palette,
}

const SLATE: Palette = Palette { ground: (0x2b, 0x35, 0x40), field: (0x4a, 0x5a, 0x6a), rosette: (0x6b, 0x7d, 0x8c), band: (0x1a, 0x21, 0x29), line: (0xc9, 0xd3, 0xdb) };
const SLATE_TEAL: Palette = Palette { ground: (0x24, 0x38, 0x3e), field: (0x3d, 0x63, 0x6a), rosette: (0x5f, 0x93, 0x96), band: (0x14, 0x23, 0x27), line: (0xcf, 0xe3, 0xe0) };
const TEAL: Palette = Palette { ground: (0x0f, 0x3d, 0x3e), field: (0x1d, 0x6f, 0x6c), rosette: (0x3f, 0xa3, 0x96), band: (0x07, 0x24, 0x25), line: (0xd8, 0xf0, 0xe8) };
const TEAL_LAPIS: Palette = Palette { ground: (0x13, 0x2f, 0x4c), field: (0x1f, 0x5f, 0x7a), rosette: (0x2e, 0x9c, 0x9a), band: (0x0a, 0x1a, 0x2d), line: (0xdc, 0xec, 0xf2) };
const LAPIS: Palette = Palette { ground: (0x14, 0x23, 0x5c), field: (0x26, 0x47, 0x9e), rosette: (0x3b, 0x6f, 0xc9), band: (0x0a, 0x12, 0x36), line: (0xe2, 0xe8, 0xf8) };
const LAPIS_GOLD: Palette = Palette { ground: (0x12, 0x1c, 0x52), field: (0x23, 0x3f, 0x96), rosette: (0xc9, 0x9a, 0x3b), band: (0x0b, 0x10, 0x30), line: (0xe8, 0xc9, 0x6a) };
const GOLD: Palette = Palette { ground: (0x2a, 0x1c, 0x0c), field: (0x8a, 0x62, 0x1e), rosette: (0xd9, 0xa8, 0x3e), band: (0x1e, 0x12, 0x05), line: (0xf6, 0xdc, 0x8a) };

/// The recipe for a tier; a tier past the top is Grandmaster.
pub fn recipe(tier: usize) -> Recipe {
    let r = |points, k, double, interlace, rosettes, rings, palette| Recipe { points, k, double, interlace, rosettes, rings, palette };
    match tier.min(TIERS - 1) {
        0 => r(8, 3, None, false, &[], 1, SLATE),
        1 => r(8, 3, None, true, &[(0.42, 3)], 1, SLATE_TEAL),
        2 => r(12, 4, None, true, &[(0.44, 4)], 2, TEAL),
        3 => r(12, 5, None, true, &[(0.46, 4)], 3, TEAL_LAPIS),
        4 => r(12, 5, Some(4), true, &[(0.46, 4)], 3, LAPIS),
        5 => r(16, 6, Some(4), true, &[(0.48, 6)], 3, LAPIS_GOLD),
        _ => r(16, 7, Some(5), true, &[(0.5, 6), (0.24, 6)], 3, GOLD),
    }
}

/// The main star's tip radius, inside the rings.
const STAR_RADIUS: f64 = 0.8;
/// Ring radii, outermost first; a tier uses the first `rings` of them.
const RINGS: [f64; 3] = [0.96, 0.88, 0.84];
/// Segments in a ring's polyline.
const RING_SEGMENTS: usize = 96;

/// A tier's badge in the unit space.
#[derive(Debug, Clone, PartialEq)]
pub struct Badge {
    pub fills: Vec<Fill>,
    pub strokes: Vec<Stroke>,
    pub palette: Palette,
}

impl Badge {
    /// Every stroke as straight segments, for a `Canvas`.
    pub fn segments(&self) -> Vec<(Point, Point)> {
        let mut out = Vec::new();
        for s in &self.strokes {
            out.extend(s.points.windows(2).map(|w| (w[0], w[1])));
            if s.closed && s.points.len() > 2 {
                out.push((s.points[s.points.len() - 1], s.points[0]));
            }
        }
        out
    }
}

/// Builds a tier's badge.
pub fn badge(tier: usize) -> Badge {
    let rc = recipe(tier);
    let n = rc.points;
    let mut fills = Vec::new();
    let mut strokes = Vec::new();
    // Tips point straight up.
    let up = PI / 2.0;
    for &radius in &RINGS[..rc.rings] {
        let ring = (0..RING_SEGMENTS).map(|i| polar(radius, 2.0 * PI * i as f64 / RING_SEGMENTS as f64 + up)).collect();
        strokes.push(Stroke { points: ring, closed: true, role: Role::Ring });
    }
    let rosettes: Vec<Vec<Point>> = rc.rosettes.iter().enumerate().map(|(i, &(radius, k))| hankin_star(n, radius, up + PI * (i + 1) as f64 / n as f64, star_contact(n, k)).expect("recipe contact in bounds")).collect();
    let mut star = |k: usize, turn: f64, strokes: &mut Vec<Stroke>| {
        let outline = hankin_star(n, STAR_RADIUS, up + turn, star_contact(n, k)).expect("recipe contact in bounds");
        fills.push(Fill { points: outline.clone(), color: rc.palette.field });
        if !rc.interlace {
            strokes.push(Stroke { points: outline, closed: true, role: Role::Star });
            return;
        }
        for i in 0..n {
            let (a, b) = (polar(STAR_RADIUS, up + turn + 2.0 * PI * i as f64 / n as f64), polar(STAR_RADIUS, up + turn + 2.0 * PI * (i + k) as f64 / n as f64));
            // Strapwork stops where it meets the rosette, so the chord splits in two there.
            match rosettes.first().and_then(|r| clip(a, b, r)) {
                Some((s, t)) => {
                    strokes.push(Stroke { points: vec![a, lerp(a, b, s)], closed: false, role: Role::Star });
                    strokes.push(Stroke { points: vec![lerp(a, b, t), b], closed: false, role: Role::Star });
                }
                None => strokes.push(Stroke { points: vec![a, b], closed: false, role: Role::Star }),
            }
        }
    };
    if let Some(k) = rc.double {
        star(k, PI / n as f64, &mut strokes);
    }
    star(rc.k, 0.0, &mut strokes);
    for outline in rosettes {
        fills.push(Fill { points: outline.clone(), color: rc.palette.rosette });
        strokes.push(Stroke { points: outline, closed: true, role: Role::Rosette });
    }
    Badge { fills, strokes, palette: rc.palette }
}

/// The first and last places, as fractions along a→b, where the segment crosses a closed polygon.
fn clip(a: Point, b: Point, poly: &[Point]) -> Option<(f64, f64)> {
    let d = (b.0 - a.0, b.1 - a.1);
    let mut hits = Vec::new();
    for (i, &p) in poly.iter().enumerate() {
        let q = poly[(i + 1) % poly.len()];
        let e = (q.0 - p.0, q.1 - p.1);
        let den = d.0 * e.1 - d.1 * e.0;
        if den.abs() < 1e-12 {
            continue;
        }
        let t = ((p.0 - a.0) * e.1 - (p.1 - a.1) * e.0) / den;
        let u = ((p.0 - a.0) * d.1 - (p.1 - a.1) * d.0) / den;
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
            hits.push(t);
        }
    }
    let first = hits.iter().copied().fold(f64::INFINITY, f64::min);
    let last = hits.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (first < last).then_some((first, last))
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

fn polar(r: f64, a: f64) -> Point {
    (r * a.cos(), r * a.sin())
}

fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

/// A coordinate to two places, with no negative zero so equal shapes print equally.
fn num(v: f64) -> String {
    let s = format!("{v:.2}");
    if s == "-0.00" { "0.00".into() } else { s }
}

/// An SVG path's `d` for a polyline mapped to the badge's centre and radius.
fn path(points: &[Point], closed: bool, cx: f64, cy: f64, r: f64) -> String {
    let mut d = String::new();
    for (i, p) in points.iter().enumerate() {
        let _ = write!(d, "{}{} {}", if i == 0 { "M" } else { " L" }, num(cx + p.0 * r), num(cy - p.1 * r));
    }
    if closed {
        d.push_str(" Z");
    }
    d
}

/// A tier's badge as an SVG `<g>`, centred on (cx, cy) with radius r: a ground disc, fills, then strapwork (a wide band stroke under a narrow line stroke).
pub fn svg(tier: usize, cx: f64, cy: f64, r: f64) -> String {
    let b = badge(tier);
    let p = b.palette;
    let mut out = String::from("<g>");
    let _ = write!(out, "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{}\"/>", num(cx), num(cy), num(r), hex(p.ground));
    for f in &b.fills {
        let _ = write!(out, "<path d=\"{}\" fill=\"{}\"/>", path(&f.points, true, cx, cy, r), hex(f.color));
    }
    let _ = write!(out, "<g fill=\"none\" stroke-linejoin=\"round\" stroke-linecap=\"round\">");
    for (color, width) in [(p.band, 0.06), (p.line, 0.02)] {
        let _ = write!(out, "<g stroke=\"{}\" stroke-width=\"{}\">", hex(color), num(width * r));
        for s in &b.strokes {
            let _ = write!(out, "<path d=\"{}\"/>", path(&s.points, s.closed, cx, cy, r));
        }
        out.push_str("</g>");
    }
    out.push_str("</g></g>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-9;

    fn dist(p: Point) -> f64 {
        p.0.hypot(p.1)
    }

    fn rotate(p: Point, a: f64) -> Point {
        (p.0 * a.cos() - p.1 * a.sin(), p.0 * a.sin() + p.1 * a.cos())
    }

    #[test]
    fn star_has_n_equidistant_tips_with_n_fold_symmetry() {
        for n in [8, 12, 16] {
            let (lo, hi) = contact_bounds(n);
            let star = hankin_star(n, 0.8, 0.3, (lo + hi) / 2.0).unwrap();
            assert_eq!(star.len(), 2 * n);
            let tips: Vec<Point> = star.iter().step_by(2).copied().collect();
            assert_eq!(tips.len(), n);
            for &t in &tips {
                assert!((dist(t) - 0.8).abs() < EPS, "tip {t:?} off the circle");
            }
            let valley = dist(star[1]);
            assert!(valley < 0.8 - 1e-3, "valleys sit inside the tips");
            // Rotating by one step maps each point onto the next of its kind.
            for (i, &p) in star.iter().enumerate() {
                let q = rotate(p, 2.0 * PI / n as f64);
                let next = star[(i + 2) % star.len()];
                assert!((q.0 - next.0).abs() < EPS && (q.1 - next.1).abs() < EPS, "n={n} i={i}");
            }
        }
    }

    #[test]
    fn contact_angle_outside_its_bounds_is_rejected() {
        let (lo, hi) = contact_bounds(8);
        assert!(lo > 0.0 && lo < hi && hi <= PI / 2.0);
        assert!(hankin_star(8, 1.0, 0.0, lo).is_none());
        assert!(hankin_star(8, 1.0, 0.0, hi).is_none());
        assert!(hankin_star(8, 1.0, 0.0, -0.5).is_none());
        assert!(hankin_star(8, 1.0, 0.0, f64::NAN).is_none());
        assert!(hankin_star(8, 0.0, 0.0, 1.0).is_none(), "a star needs a radius");
        assert!(hankin_star(2, 1.0, 0.0, 1.0).is_none(), "a polygon needs three sides");
        assert!(hankin_star(8, 1.0, 0.0, lo + 1e-3).is_some());
    }

    #[test]
    fn contact_angle_of_pi_k_over_n_puts_the_valley_on_the_chord() {
        // At contact πk/n the ray from one tip runs to the tip k steps on, so the valley lies on that chord: the {n/k} star.
        let star = hankin_star(8, 1.0, 0.0, 3.0 * PI / 8.0).unwrap();
        let (a, b, v) = (star[0], star[6], star[1]);
        let cross = (b.0 - a.0) * (v.1 - a.1) - (b.1 - a.1) * (v.0 - a.0);
        assert!(cross.abs() < EPS, "valley {v:?} off chord {a:?}-{b:?}");
    }

    #[test]
    fn same_tier_yields_identical_svg() {
        for tier in 0..TIERS {
            assert_eq!(svg(tier, 120.0, 120.0, 100.0), svg(tier, 120.0, 120.0, 100.0));
            assert!(!svg(tier, 120.0, 120.0, 100.0).is_empty());
        }
    }

    #[test]
    fn higher_tiers_have_strictly_more_segments() {
        let counts: Vec<usize> = (0..TIERS).map(|t| badge(t).segments().len()).collect();
        assert!(counts[0] > 0);
        for w in counts.windows(2) {
            assert!(w[0] < w[1], "segment counts {counts:?}");
        }
    }

    #[test]
    fn star_points_grow_with_tier_from_8_through_12_to_16() {
        let points: Vec<usize> = (0..TIERS).map(|t| recipe(t).points).collect();
        assert_eq!(points, [8, 8, 12, 12, 12, 16, 16]);
    }

    #[test]
    fn tier_past_the_top_clamps_to_grandmaster() {
        assert_eq!(svg(99, 50.0, 50.0, 40.0), svg(TIERS - 1, 50.0, 50.0, 40.0));
    }

    #[test]
    fn svg_is_a_vector_group_with_no_text_or_raster() {
        for tier in 0..TIERS {
            let s = svg(tier, 120.0, 120.0, 100.0);
            assert!(s.starts_with("<g") && s.trim_end().ends_with("</g>"), "tier {tier}");
            for banned in ["<text", "<image", "<foreignObject", "data:", "href"] {
                assert!(!s.contains(banned), "tier {tier} has {banned}");
            }
            assert!(!s.contains("NaN") && !s.contains("inf"), "tier {tier} has a bad number");
        }
    }

    #[test]
    fn svg_scales_to_the_given_centre_and_radius() {
        let s = svg(0, 200.0, 150.0, 50.0);
        assert!(s.contains("cx=\"200.00\" cy=\"150.00\" r=\"50.00\""), "{s}");
    }

    #[test]
    fn strokes_stay_inside_the_unit_disc() {
        for tier in 0..TIERS {
            for (a, b) in badge(tier).segments() {
                assert!(dist(a) <= 1.0 + EPS && dist(b) <= 1.0 + EPS, "tier {tier}");
            }
        }
    }

    #[test]
    fn interlace_stops_at_the_rosette() {
        for tier in 0..TIERS {
            let rc = recipe(tier);
            let Some(&(radius, _)) = rc.rosettes.first() else { continue };
            // A chord that passes inside the rosette's tips splits in two there; one that clears them stays whole.
            let n = rc.points;
            let chords = |k: usize| if STAR_RADIUS * (PI * k as f64 / n as f64).cos() < radius { 2 * n } else { n };
            let want = chords(rc.k) + rc.double.map_or(0, chords);
            let star: Vec<Stroke> = badge(tier).strokes.into_iter().filter(|s| s.role == Role::Star).collect();
            assert_eq!(star.len(), want, "tier {tier}");
            for p in star.iter().flat_map(|s| s.points.iter().copied()) {
                assert!(dist(p) > radius * 0.6, "tier {tier} point {p:?} inside the rosette");
            }
        }
    }

    #[test]
    #[ignore = "writes a preview SVG of all tiers to $BADGE_PREVIEW"]
    fn preview_all_tiers() {
        let path = std::env::var("BADGE_PREVIEW").expect("set BADGE_PREVIEW");
        let mut out = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1680\" height=\"260\" viewBox=\"0 0 1680 260\"><rect width=\"1680\" height=\"260\" fill=\"#f4f1ea\"/>");
        for tier in 0..TIERS {
            out.push_str(&svg(tier, 120.0 + 240.0 * tier as f64, 130.0, 110.0));
        }
        out.push_str("</svg>");
        std::fs::write(path, out).unwrap();
    }
}
