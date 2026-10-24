//! Badge art: a deterministic Islamic ornament per tier, in three historical idioms — Mamluk/Seljuk
//! carved roundel (tiers 0-1), Moorish zellij tilework (tiers 2-4), Persian girih strapwork (tiers 5-6) —
//! as strokes/fills in a unit space and as an SVG fragment.
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

/// What a stroke is part of; `svg` renders strapwork (`Strap`, `Rosette`) as a wide band under a narrow
/// line, and rim and grout (`Ring`, `Tile`) as one thin line. The badge view colours by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Ring,
    Tile,
    Strap,
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

/// A tier's representative colours, used for the badge ground and strapwork and reused by the card for
/// its own chrome (the ground tint, the frame band and rule, the badge's halo and shadow).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// The disc behind the pattern; the darkest, recessed tone.
    pub ground: Rgb,
    /// The main motif's field; the badge's dominant tone.
    pub field: Rgb,
    /// The central rosette's fill.
    pub rosette: Rgb,
    /// The wide stroke under strapwork, and the rim/grout lines.
    pub band: Rgb,
    /// The narrow stroke on top of strapwork.
    pub line: Rgb,
}

// Mamluk / Seljuk carved roundel (tiers 0-1): deep-relief interlace in one material, read through a
// single hue's light and shadow, not colour.
const MAMLUK_IVORY: Palette = Palette { ground: (0x5b, 0x4d, 0x36), field: (0xcf, 0xbf, 0x95), rosette: (0xe6, 0xda, 0xb8), band: (0x35, 0x2b, 0x1c), line: (0xf8, 0xf1, 0xdf) };
const MAMLUK_CEDAR: Palette = Palette { ground: (0x3a, 0x1e, 0x11), field: (0x9c, 0x56, 0x30), rosette: (0xc9, 0x8a, 0x58), band: (0x22, 0x10, 0x08), line: (0xf1, 0xd9, 0xb6) };

// Moorish zellij (tiers 2-4): an 8-fold khatam rosette in a tiled field of small cut-tile pieces, with
// broad interlaced bands over the top.
const ZELLIJ_TURQUOISE: Palette = Palette { ground: (0x0c, 0x38, 0x3a), field: (0x1c, 0x8f, 0x8a), rosette: (0xf3, 0xec, 0xd8), band: (0x06, 0x1c, 0x1e), line: (0xf3, 0xec, 0xd8) };
const ZELLIJ_COBALT: Palette = Palette { ground: (0x0a, 0x17, 0x3f), field: (0x20, 0x45, 0x97), rosette: (0xe7, 0xcc, 0x86), band: (0x05, 0x0b, 0x24), line: (0xe7, 0xcc, 0x86) };
const ZELLIJ_RICH: Palette = Palette { ground: (0x07, 0x0d, 0x22), field: (0x19, 0x2c, 0x78), rosette: (0xd3, 0x8a, 0x27), band: (0x04, 0x04, 0x09), line: (0xf3, 0xe7, 0xc2) };

const ZELLIJ_TILES_2: [Rgb; 2] = [(0x1c, 0x8f, 0x8a), (0xe9, 0xe2, 0xcf)];
const ZELLIJ_TILES_3: [Rgb; 4] = [(0x20, 0x45, 0x97), (0x1c, 0x8f, 0x8a), (0xe7, 0xcc, 0x86), (0x16, 0x16, 0x1c)];
const ZELLIJ_TILES_4: [Rgb; 6] = [(0x19, 0x2c, 0x78), (0x1c, 0x8f, 0x8a), (0x3e, 0x7d, 0x3a), (0xd3, 0x8a, 0x27), (0x14, 0x14, 0x18), (0xef, 0xe8, 0xd6)];

// Persian girih (tiers 5-6): 10-fold decagon rosettes with long interlaced strapwork, turquoise and
// lapis with gold, richest at the top.
const GIRIH_TURQUOISE: Palette = Palette { ground: (0x08, 0x12, 0x2c), field: (0x24, 0x4d, 0xb8), rosette: (0x2a, 0x9d, 0x93), band: (0x04, 0x08, 0x18), line: (0xdd, 0xb6, 0x4e) };
const GIRIH_GOLD: Palette = Palette { ground: (0x06, 0x0a, 0x20), field: (0x2a, 0x35, 0x8c), rosette: (0xe3, 0xb8, 0x3c), band: (0x03, 0x04, 0x10), line: (0xf6, 0xda, 0x7e) };

/// Ring radii, outermost first; a badge uses the first few of them as a rim.
const RINGS: [f64; 3] = [0.97, 0.91, 0.855];
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

/// Builds a tier's badge; a tier past the top is Grandmaster.
pub fn badge(tier: usize) -> Badge {
    match tier.min(TIERS - 1) {
        0 => carved(MAMLUK_IVORY, 1),
        1 => carved(MAMLUK_CEDAR, 2),
        2 => zellij(ZELLIJ_TURQUOISE, &ZELLIJ_TILES_2, 1),
        3 => zellij(ZELLIJ_COBALT, &ZELLIJ_TILES_3, 2),
        4 => zellij(ZELLIJ_RICH, &ZELLIJ_TILES_4, 3),
        5 => girih(GIRIH_TURQUOISE, 1),
        _ => girih(GIRIH_GOLD, 2),
    }
}

/// The rim, outermost first, as `richness` concentric ring strokes.
fn rings(richness: usize) -> Vec<Stroke> {
    RINGS[..richness.min(RINGS.len())].iter().map(|&radius| Stroke { points: ring_points(radius), closed: true, role: Role::Ring }).collect()
}

fn ring_points(radius: f64) -> Vec<Point> {
    (0..RING_SEGMENTS).map(|i| polar(radius, 2.0 * PI * i as f64 / RING_SEGMENTS as f64 + PI / 2.0)).collect()
}

/// A plain n-gon, flat-sided and unpointed, its first vertex at angle `rotation`.
fn regular_polygon(n: usize, radius: f64, rotation: f64) -> Vec<Point> {
    (0..n).map(|i| polar(radius, rotation + 2.0 * PI * i as f64 / n as f64)).collect()
}

/// A scalloped guilloche ring at mean radius `r0`, its edge gently waving `waves` times: a finely
/// beaded border rather than a pointed star, the standard trim of a certificate or a coin's rim.
fn rope(r0: f64, amp: f64, waves: usize, phase: f64) -> Vec<Point> {
    const SEGS_PER_WAVE: usize = 8;
    let segs = waves * SEGS_PER_WAVE;
    (0..segs)
        .map(|i| {
            let t = 2.0 * PI * i as f64 / segs as f64;
            polar(r0 + amp * (waves as f64 * t + phase).cos(), t)
        })
        .collect()
}

/// A Mamluk/Seljuk carved roundel: a flat octagonal boss in the material's raised tone, with a carved
/// inset line just inside it standing for the groove, and — past Novice — a raised centre medallion.
fn carved(palette: Palette, richness: usize) -> Badge {
    let up = PI / 2.0;
    let mut fills = Vec::new();
    let mut strokes = rings(richness);

    let outline = regular_polygon(8, 0.8, up + PI / 8.0);
    fills.push(Fill { points: outline.clone(), color: palette.field });
    strokes.push(Stroke { points: outline.clone(), closed: true, role: Role::Strap });
    let inset: Vec<Point> = outline.iter().map(|&(x, y)| (x * 0.9, y * 0.9)).collect();
    strokes.push(Stroke { points: inset, closed: true, role: Role::Ring });

    if richness > 1 {
        let boss = regular_polygon(8, 0.36, up + PI / 8.0);
        fills.push(Fill { points: boss.clone(), color: palette.rosette });
        strokes.push(Stroke { points: boss, closed: true, role: Role::Rosette });
    }
    Badge { fills, strokes, palette }
}

/// A closed annular wedge from `a0` to `a1`, outer arc then inner arc, approximated in `segs` steps.
fn wedge(r0: f64, r1: f64, a0: f64, a1: f64, segs: usize) -> Vec<Point> {
    let mut out = Vec::with_capacity(2 * segs + 2);
    for i in 0..=segs {
        out.push(polar(r1, a0 + (a1 - a0) * i as f64 / segs as f64));
    }
    for i in 0..=segs {
        out.push(polar(r0, a1 - (a1 - a0) * i as f64 / segs as f64));
    }
    out
}

/// A Moorish zellij medallion: a flat octagonal centrepiece, concentric rings of small cut-tile wedges
/// (finer and more colourful outward with `richness`), and one or two scalloped guilloche borders on top.
fn zellij(palette: Palette, tiles: &'static [Rgb], richness: usize) -> Badge {
    let up = PI / 2.0;
    let mut fills = Vec::new();
    let mut strokes = rings(richness);

    let centre = regular_polygon(8, 0.3, up + PI / 8.0);
    fills.push(Fill { points: centre.clone(), color: palette.rosette });
    strokes.push(Stroke { points: centre, closed: true, role: Role::Rosette });

    let bands: &[(f64, f64, usize)] = match richness {
        1 => &[(0.32, 0.58, 8)],
        2 => &[(0.32, 0.54, 8), (0.56, 0.76, 12)],
        _ => &[(0.32, 0.50, 8), (0.52, 0.70, 12), (0.72, 0.84, 18)],
    };
    for (bi, &(r0, r1, n)) in bands.iter().enumerate() {
        for i in 0..n {
            let (a0, a1) = (up + 2.0 * PI * i as f64 / n as f64, up + 2.0 * PI * (i + 1) as f64 / n as f64);
            let points = wedge(r0, r1, a0, a1, 6);
            fills.push(Fill { points: points.clone(), color: tiles[(i + bi) % tiles.len()] });
            strokes.push(Stroke { points, closed: true, role: Role::Tile });
        }
    }

    strokes.push(Stroke { points: rope(0.88, 0.022, 16, 0.0), closed: true, role: Role::Strap });
    if richness > 1 {
        strokes.push(Stroke { points: rope(0.88, 0.018, 22, PI / 11.0), closed: true, role: Role::Strap });
    }
    Badge { fills, strokes, palette }
}

/// A Persian girih medallion: a flat decagon centrepiece in a tessellation of bowtie and hexagon tiles
/// — girih's own tile set, not a radiating star — under a scalloped guilloche border, richest at the top.
fn girih(palette: Palette, richness: usize) -> Badge {
    let up = PI / 2.0;
    let mut fills = Vec::new();
    let mut strokes = rings(richness + 1);

    let centre = regular_polygon(10, 0.26, up);
    fills.push(Fill { points: centre.clone(), color: palette.rosette });
    strokes.push(Stroke { points: centre, closed: true, role: Role::Rosette });

    // Bowtie tiles ringing the centrepiece, girih's own filler piece between neighbouring decagons.
    let spread = PI / 16.0;
    for i in 0..10 {
        let a = up + 2.0 * PI * i as f64 / 10.0 + PI / 10.0;
        let bowtie = vec![polar(0.28, a), polar(0.48, a - spread), polar(0.6, a), polar(0.48, a + spread)];
        fills.push(Fill { points: bowtie.clone(), color: palette.field });
        strokes.push(Stroke { points: bowtie, closed: true, role: Role::Tile });
    }

    if richness > 1 {
        // Grandmaster's outer ring: small flat hexagon tiles, girih's third piece, completing the tessellation.
        for i in 0..10 {
            let a = up + 2.0 * PI * i as f64 / 10.0;
            let hexagon: Vec<Point> = regular_polygon(6, 0.085, a).iter().map(|&(x, y)| (x + 0.72 * a.cos(), y + 0.72 * a.sin())).collect();
            fills.push(Fill { points: hexagon.clone(), color: palette.rosette });
            strokes.push(Stroke { points: hexagon, closed: true, role: Role::Tile });
        }
    }

    strokes.push(Stroke { points: rope(0.87, 0.022, 18, 0.0), closed: true, role: Role::Strap });
    if richness > 1 {
        strokes.push(Stroke { points: rope(0.79, 0.016, 26, PI / 13.0), closed: true, role: Role::Strap });
    }
    Badge { fills, strokes, palette }
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

/// A tier's badge as an SVG `<g>`, centred on (cx, cy) with radius r: a ground disc, fills, a thin
/// line for the rim and tile grout, then strapwork (a wide band stroke under a narrow line stroke).
pub fn svg(tier: usize, cx: f64, cy: f64, r: f64) -> String {
    let b = badge(tier);
    let p = b.palette;
    let mut out = String::from("<g>");
    let _ = write!(out, "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{}\"/>", num(cx), num(cy), num(r), hex(p.ground));
    for f in &b.fills {
        let _ = write!(out, "<path d=\"{}\" fill=\"{}\"/>", path(&f.points, true, cx, cy, r), hex(f.color));
    }
    let _ = write!(out, "<g fill=\"none\" stroke-linejoin=\"round\" stroke-linecap=\"round\">");
    let _ = write!(out, "<g stroke=\"{}\" stroke-width=\"{}\">", hex(p.band), num(0.012 * r));
    for s in b.strokes.iter().filter(|s| matches!(s.role, Role::Ring | Role::Tile)) {
        let _ = write!(out, "<path d=\"{}\"/>", path(&s.points, s.closed, cx, cy, r));
    }
    out.push_str("</g>");
    for (color, width) in [(p.band, 0.065), (p.line, 0.022)] {
        let _ = write!(out, "<g stroke=\"{}\" stroke-width=\"{}\">", hex(color), num(width * r));
        for s in b.strokes.iter().filter(|s| matches!(s.role, Role::Strap | Role::Rosette)) {
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

    fn distinct_colors(tier: usize) -> usize {
        let b = badge(tier);
        let mut colors: Vec<Rgb> = b.fills.iter().map(|f| f.color).collect();
        colors.sort();
        colors.dedup();
        colors.len()
    }

    #[test]
    fn star_has_n_equidistant_tips_with_n_fold_symmetry() {
        for n in [8, 10, 12, 16] {
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
    fn carved_tiers_read_as_one_material() {
        // Novice has only the raised strap tone; Apprentice adds a single boss tone.
        assert_eq!(distinct_colors(0), 1, "Novice should be one material tone");
        assert_eq!(distinct_colors(1), 2, "Apprentice adds only the boss tone");
    }

    #[test]
    fn zellij_tiers_grow_more_colourful_with_tier() {
        let counts: Vec<usize> = (2..=4).map(distinct_colors).collect();
        assert!(counts[0] < counts[1] && counts[1] < counts[2], "zellij colour counts {counts:?}");
    }

    #[test]
    fn girih_tiers_have_a_flat_tenfold_centrepiece_and_ten_bowtie_tiles() {
        for tier in [5, 6] {
            let b = badge(tier);
            let rosette = b.strokes.iter().find(|s| s.role == Role::Rosette).expect("a rosette");
            assert_eq!(rosette.points.len(), 10, "a flat decagon has 10 points, not a star's 20");
            let tiles = b.strokes.iter().filter(|s| s.role == Role::Tile).count();
            assert!(tiles >= 10, "tier {tier} has only {tiles} tiles");
        }
        // Grandmaster adds a ring of ten hexagon tiles and a second guilloche border.
        assert_eq!(badge(6).strokes.iter().filter(|s| s.role == Role::Tile).count(), 20);
        assert_eq!(badge(6).strokes.iter().filter(|s| s.role == Role::Strap).count(), 2);
    }

    #[test]
    fn no_badge_shape_has_a_sharp_outward_point() {
        // Every closed fill should be a convex-ish polygon or wedge, not a star with deep inward valleys:
        // each vertex stays within a modest distance of the shape's own centroid.
        for tier in 0..TIERS {
            for f in &badge(tier).fills {
                let n = f.points.len() as f64;
                let (cx, cy) = f.points.iter().fold((0.0, 0.0), |(sx, sy), p| (sx + p.0 / n, sy + p.1 / n));
                let radii: Vec<f64> = f.points.iter().map(|&(x, y)| dist((x - cx, y - cy))).collect();
                let (min, max) = (radii.iter().copied().fold(f64::INFINITY, f64::min), radii.iter().copied().fold(0.0, f64::max));
                assert!(max < min * 3.0 || min < 1e-6, "tier {tier} fill has a spiky point: {radii:?}");
            }
        }
    }

    #[test]
    fn richer_tiers_within_a_style_have_more_geometry() {
        let total = |t: usize| badge(t).fills.len() + badge(t).strokes.len();
        assert!(total(0) < total(1), "carved: {} vs {}", total(0), total(1));
        assert!(total(2) < total(3) && total(3) < total(4), "zellij: {} {} {}", total(2), total(3), total(4));
        assert!(total(5) < total(6), "girih: {} vs {}", total(5), total(6));
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

    #[test]
    #[ignore = "writes a preview PNG of all tiers to $BADGE_PREVIEW_PNG"]
    fn preview_all_tiers_png() {
        let path = std::env::var("BADGE_PREVIEW_PNG").expect("set BADGE_PREVIEW_PNG");
        let mut svg_doc = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1680\" height=\"260\" viewBox=\"0 0 1680 260\"><rect width=\"1680\" height=\"260\" fill=\"#f4f1ea\"/>");
        for tier in 0..TIERS {
            svg_doc.push_str(&svg(tier, 120.0 + 240.0 * tier as f64, 130.0, 110.0));
        }
        svg_doc.push_str("</svg>");
        let tree = resvg::usvg::Tree::from_str(&svg_doc, &resvg::usvg::Options::default()).unwrap();
        let mut pixmap = resvg::tiny_skia::Pixmap::new(1680, 260).unwrap();
        resvg::render(&tree, resvg::tiny_skia::Transform::default(), &mut pixmap.as_mut());
        pixmap.save_png(path).unwrap();
    }
}
