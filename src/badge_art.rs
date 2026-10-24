//! Badge art: a deterministic Islamic geometric medallion per tier, in three historical idioms —
//! Mamluk/Seljuk carving (tiers 0-1), Moorish zellij (2-4) and Persian tilework (5-6). Every badge
//! has the same structure, a bevelled rim, a border band and an all-over tessellated field, and the
//! tiers grow from plain carved stone to jewelled gold.

use std::f64::consts::PI;
use std::fmt::Write;

/// A point in the unit badge space: centre at the origin, y up, the badge rim at radius 1.
pub type Point = (f64, f64);

/// The number of tiers, Novice (0) to Grandmaster (6).
pub const TIERS: usize = crate::badges::TIERS.len();

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

/// What a stroke is part of; the badge view colours by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The rim and the edges of the band and the field.
    Rim,
    /// Ornament in the border band.
    Band,
    /// The outline of a tile in the field.
    Tile,
}

/// A closed polyline in the unit space, with its colour and its width in unit-space terms.
#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub points: Vec<Point>,
    pub role: Role,
    pub color: Rgb,
    pub width: f64,
}

/// A filled convex polygon in the unit space, painted in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    pub points: Vec<Point>,
    pub color: Rgb,
}

/// A tier's colours: the badge's rim material, and the tones the card reuses for its own chrome (the
/// ground tint, the frame band and its stars, the badge's halo and shadow).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// The darkest tone; the card's frame band.
    pub ground: Rgb,
    /// The dominant tone; the card's accent and lattice tint.
    pub field: Rgb,
    /// The shadow tone.
    pub band: Rgb,
    /// The rim's material.
    pub metal: Metal,
}

/// A rim material's light, mid and dark tones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metal {
    pub light: Rgb,
    pub mid: Rgb,
    pub dark: Rgb,
}

const SANDSTONE: Metal = Metal { light: (0xee, 0xe2, 0xc4), mid: (0xcc, 0xb5, 0x8a), dark: (0x7e, 0x67, 0x45) };
const CEDAR: Metal = Metal { light: (0xdc, 0xa3, 0x6e), mid: (0xa3, 0x63, 0x36), dark: (0x52, 0x2a, 0x13) };
const BRONZE: Metal = Metal { light: (0xf2, 0xc4, 0x90), mid: (0xb8, 0x76, 0x42), dark: (0x66, 0x39, 0x18) };
const SILVER: Metal = Metal { light: (0xfa, 0xfb, 0xfc), mid: (0xbf, 0xc6, 0xce), dark: (0x63, 0x6c, 0x77) };
const GOLD: Metal = Metal { light: (0xfe, 0xec, 0xab), mid: (0xd8, 0xaa, 0x46), dark: (0x7f, 0x58, 0x12) };
const PEARL: Metal = Metal { light: (0xff, 0xff, 0xff), mid: (0xec, 0xe7, 0xdc), dark: (0x95, 0x8e, 0x82) };

const COBALT: Rgb = (0x1f, 0x3f, 0x93);
const TURQUOISE: Rgb = (0x1b, 0x95, 0x8d);
const GREEN: Rgb = (0x2c, 0x7a, 0x4d);
const OCHRE: Rgb = (0xd0, 0x8b, 0x2c);
const JET: Rgb = (0x1c, 0x1d, 0x23);
const CREAM: Rgb = (0xf3, 0xed, 0xde);
const LAPIS: Rgb = (0x23, 0x46, 0xa4);
const DEEP_LAPIS: Rgb = (0x15, 0x28, 0x68);

/// The tessellation that fills the field, with its scale.
#[derive(Debug, Clone, Copy)]
enum Pattern {
    /// Squares turned on point, this far apart: the plainest carved diaper.
    Diamonds(f64),
    /// Hexagons of this circumradius.
    Honeycomb(f64),
    /// Octagons and squares (4.8.8), octagon centres this far apart: the zellij staple.
    Octagons(f64),
    /// A decagon ringed twice by pentagons and thin rhombi, out to this radius: a Persian dissection.
    Decagons(f64),
}

/// The ornament in the border band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Border {
    Plain,
    Fillet,
    Beads,
    Rope,
    BeadsAndFillet,
    Jewels,
    Crown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finish {
    /// Raised strapwork round recessed panels, read by light and shadow.
    Carved,
    /// Flush glazed tiles in a strapwork ground.
    Glazed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Centre {
    Raised,
    Gilded,
    Jewelled,
}

/// Everything that sets one tier's badge apart.
struct Spec {
    border: Border,
    pattern: Pattern,
    finish: Finish,
    /// The strapwork: the field showing between the tiles.
    strap: Rgb,
    /// Tile colours by kind (two alternating, one secondary).
    tiles: [Rgb; 3],
    /// A smaller tile set into each whole tile, by kind.
    inlay: Option<[Rgb; 3]>,
    centre: Centre,
    /// The centrepiece's inlay.
    jewel: Rgb,
    /// Half the strapwork's width.
    gap: f64,
    gloss: f64,
    palette: Palette,
}

fn spec(tier: usize) -> Spec {
    match tier.min(TIERS - 1) {
        0 => Spec {
            border: Border::Plain,
            pattern: Pattern::Diamonds(0.38),
            finish: Finish::Carved,
            strap: SANDSTONE.light,
            tiles: [SANDSTONE.mid; 3],
            inlay: Some([SANDSTONE.light; 3]),
            centre: Centre::Raised,
            jewel: SANDSTONE.mid,
            gap: 0.016,
            gloss: 0.1,
            palette: Palette { ground: (0x5a, 0x48, 0x2f), field: (0x8f, 0x74, 0x4c), band: (0x3b, 0x2f, 0x1e), metal: SANDSTONE },
        },
        1 => Spec {
            border: Border::Fillet,
            pattern: Pattern::Honeycomb(0.15),
            finish: Finish::Carved,
            strap: CEDAR.light,
            tiles: [CEDAR.mid; 3],
            inlay: Some([CEDAR.light; 3]),
            centre: Centre::Raised,
            jewel: CEDAR.mid,
            gap: 0.014,
            gloss: 0.12,
            palette: Palette { ground: (0x3f, 0x22, 0x12), field: (0x9a, 0x58, 0x2c), band: (0x26, 0x13, 0x09), metal: CEDAR },
        },
        2 => Spec {
            border: Border::Beads,
            pattern: Pattern::Octagons(0.4),
            finish: Finish::Glazed,
            strap: CREAM,
            tiles: [TURQUOISE, TURQUOISE, COBALT],
            inlay: None,
            centre: Centre::Gilded,
            jewel: COBALT,
            gap: 0.013,
            gloss: 0.22,
            palette: Palette { ground: (0x0e, 0x3c, 0x3a), field: (0x17, 0x86, 0x7f), band: (0x07, 0x22, 0x21), metal: BRONZE },
        },
        3 => Spec {
            border: Border::Rope,
            pattern: Pattern::Octagons(0.34),
            finish: Finish::Glazed,
            strap: CREAM,
            tiles: [COBALT, TURQUOISE, OCHRE],
            inlay: None,
            centre: Centre::Gilded,
            jewel: TURQUOISE,
            gap: 0.012,
            gloss: 0.25,
            palette: Palette { ground: (0x13, 0x24, 0x57), field: (0x25, 0x4a, 0xa8), band: (0x0a, 0x13, 0x30), metal: SILVER },
        },
        4 => Spec {
            border: Border::BeadsAndFillet,
            pattern: Pattern::Octagons(0.32),
            finish: Finish::Glazed,
            strap: JET,
            tiles: [GREEN, COBALT, CREAM],
            inlay: Some([TURQUOISE, OCHRE, OCHRE]),
            centre: Centre::Gilded,
            jewel: GREEN,
            gap: 0.011,
            gloss: 0.28,
            palette: Palette { ground: (0x12, 0x2e, 0x22), field: (0x2a, 0x6e, 0x48), band: (0x08, 0x18, 0x11), metal: GOLD },
        },
        5 => Spec {
            border: Border::Jewels,
            pattern: Pattern::Decagons(0.86),
            finish: Finish::Glazed,
            strap: GOLD.mid,
            tiles: [LAPIS, TURQUOISE, CREAM],
            inlay: Some([TURQUOISE, CREAM, LAPIS]),
            centre: Centre::Gilded,
            jewel: LAPIS,
            gap: 0.011,
            gloss: 0.3,
            palette: Palette { ground: (0x10, 0x1e, 0x4c), field: (0x24, 0x47, 0xa8), band: (0x08, 0x0f, 0x2a), metal: GOLD },
        },
        _ => Spec {
            border: Border::Crown,
            pattern: Pattern::Decagons(0.86),
            finish: Finish::Glazed,
            strap: GOLD.mid,
            tiles: [DEEP_LAPIS, TURQUOISE, CREAM],
            inlay: Some([TURQUOISE, DEEP_LAPIS, LAPIS]),
            centre: Centre::Jewelled,
            jewel: DEEP_LAPIS,
            gap: 0.01,
            gloss: 0.32,
            palette: Palette { ground: (0x0d, 0x16, 0x38), field: (0x9a, 0x70, 0x1c), band: (0x05, 0x0a, 0x1e), metal: GOLD },
        },
    }
}

/// The field's radius, inside the border band.
const FIELD: f64 = 0.8;
/// The border band's outer radius, where the rim begins.
const BAND: f64 = 0.92;
/// Sides of the polygon that stands for a circle: divisible by every pattern's symmetry (4, 6, 10).
const CIRCLE: usize = 120;

/// A tier's badge in the unit space.
#[derive(Debug, Clone, PartialEq)]
pub struct Badge {
    pub fills: Vec<Fill>,
    pub strokes: Vec<Stroke>,
    pub palette: Palette,
    /// The sheen's opacity: matte stone low, glaze and gold higher.
    pub gloss: f64,
}

impl Badge {
    /// Every stroke as straight segments with its role, for a `Canvas`.
    pub fn segments(&self) -> Vec<(Point, Point, Role)> {
        let mut out = Vec::new();
        for s in &self.strokes {
            out.extend(s.points.windows(2).map(|w| (w[0], w[1], s.role)));
            if s.points.len() > 2 {
                out.push((s.points[s.points.len() - 1], s.points[0], s.role));
            }
        }
        out
    }
}

/// A tier's colours, without building its geometry.
pub fn palette(tier: usize) -> Palette {
    spec(tier).palette
}

/// Builds a tier's badge; a tier past the top is Grandmaster.
pub fn badge(tier: usize) -> Badge {
    let sp = spec(tier);
    let mut fills = Vec::new();
    let mut strokes = Vec::new();
    border(&sp, &mut fills, &mut strokes);
    field(&sp, &mut fills, &mut strokes);
    for radius in [1.0, BAND, FIELD] {
        strokes.push(Stroke { points: circle((0.0, 0.0), radius, CIRCLE), role: Role::Rim, color: sp.palette.metal.dark, width: 0.008 });
    }
    // Clipping a highlight to a sliver of a tile at the field's edge can leave nothing.
    fills.retain(|f| f.points.len() >= 3 && area(&f.points) > 1e-12);
    Badge { fills, strokes, palette: sp.palette, gloss: sp.gloss }
}

/// A tile of a tessellation: its outline, its colour kind, and whether it sits at the centre.
struct Tile {
    points: Vec<Point>,
    kind: usize,
    centre: bool,
}

/// The tiles of a pattern that can reach the field, centred so one tile sits on the origin.
fn tiles(pattern: Pattern) -> Vec<Tile> {
    let reach = FIELD + 0.3;
    let mut out = Vec::new();
    let mut put = |c: Point, points: Vec<Point>, kind: usize, centre: bool| {
        if c.0.hypot(c.1) <= reach {
            out.push(Tile { points: shift(&points, c), kind, centre });
        }
    };
    match pattern {
        Pattern::Diamonds(p) => {
            let n = (reach / p).ceil() as i32 + 1;
            for j in -n..=n {
                for i in -n..=n {
                    let (x, y) = (i as f64 * p, j as f64 * p);
                    put((x, y), regular_polygon(4, p / 2.0, 0.0), 0, i == 0 && j == 0);
                    put((x + p / 2.0, y + p / 2.0), regular_polygon(4, p / 2.0, 0.0), 0, false);
                }
            }
        }
        Pattern::Honeycomb(s) => {
            let n = (2.0 * reach / (3f64.sqrt() * s)).ceil() as i32 + 1;
            for r in -n..=n {
                for q in -n..=n {
                    let c = (3f64.sqrt() * s * (q as f64 + r as f64 / 2.0), 1.5 * s * r as f64);
                    put(c, regular_polygon(6, s, PI / 2.0), 0, q == 0 && r == 0);
                }
            }
        }
        Pattern::Octagons(p) => {
            let side = p / (1.0 + 2f64.sqrt());
            let n = (reach / p).ceil() as i32 + 1;
            for j in -n..=n {
                for i in -n..=n {
                    let (x, y) = (i as f64 * p, j as f64 * p);
                    put((x, y), regular_polygon(8, side / (2.0 * (PI / 8.0).sin()), PI / 8.0), (i + j).rem_euclid(2) as usize, i == 0 && j == 0);
                    put((x + p / 2.0, y + p / 2.0), regular_polygon(4, side / 2f64.sqrt(), 0.0), 2, false);
                }
            }
        }
        Pattern::Decagons(outer) => {
            // Each ring round a decagon of side s is ten pentagons of side s with a 36°/144° rhombus
            // in every notch between them; together they make a decagon of side 2s.
            let rings = 2;
            let mut side = outer * 2.0 * (PI / 10.0).sin() / f64::from(1 << rings);
            let mut v = regular_polygon(10, side / (2.0 * (PI / 10.0).sin()), PI / 2.0);
            out.push(Tile { points: v.clone(), kind: 0, centre: true });
            for _ in 0..rings {
                let (apothem, radius) = (side / (2.0 * (PI / 5.0).tan()), side / (2.0 * (PI / 5.0).sin()));
                let pentagons: Vec<Vec<Point>> = (0..10)
                    .map(|i| {
                        let (p, q) = (v[i], v[(i + 1) % 10]);
                        let len = (q.0 - p.0).hypot(q.1 - p.1);
                        let out_n = ((q.1 - p.1) / len, -(q.0 - p.0) / len);
                        let c = ((p.0 + q.0) / 2.0 + out_n.0 * apothem, (p.1 + q.1) / 2.0 + out_n.1 * apothem);
                        shift(&regular_polygon(5, radius, out_n.1.atan2(out_n.0)), c)
                    })
                    .collect();
                let mut next = Vec::with_capacity(10);
                for i in 0..10 {
                    let (apex, apex_next, corner) = (pentagons[i][0], pentagons[(i + 1) % 10][0], v[(i + 1) % 10]);
                    let notch = if dist2(pentagons[i][1], corner) < dist2(pentagons[i][4], corner) { pentagons[i][1] } else { pentagons[i][4] };
                    let tip = (apex.0 + apex_next.0 - notch.0, apex.1 + apex_next.1 - notch.1);
                    out.push(Tile { points: ccw(vec![notch, apex, tip, apex_next]), kind: 2, centre: false });
                    next.push(tip);
                }
                for (i, p) in pentagons.into_iter().enumerate() {
                    out.push(Tile { points: p, kind: i % 2, centre: false });
                }
                v = next;
                side *= 2.0;
            }
        }
    }
    out
}

/// The field: the strapwork ground, then each tile inset by the strap's half-width, clipped to the
/// field and finished, with its outline.
fn field(sp: &Spec, fills: &mut Vec<Fill>, strokes: &mut Vec<Stroke>) {
    let m = sp.palette.metal;
    let outline = match sp.finish {
        Finish::Carved => m.dark,
        Finish::Glazed => mix(sp.strap, (0, 0, 0), 0.55),
    };
    fills.push(Fill { points: circle((0.0, 0.0), FIELD, CIRCLE), color: sp.strap });
    let bound = circle((0.0, 0.0), FIELD - sp.gap, CIRCLE);
    let edge = |points: Vec<Point>, strokes: &mut Vec<Stroke>| strokes.push(Stroke { points, role: Role::Tile, color: outline, width: 0.0045 });
    for t in tiles(sp.pattern) {
        let Some(inner) = inset(&t.points, sp.gap) else { continue };
        let piece = clip(&inner, &bound);
        if area(&piece) < 2e-4 {
            continue;
        }
        if t.centre {
            centrepiece(sp, &piece, &bound, fills);
            if sp.centre != Centre::Raised {
                edge(scale(&piece, 0.62), strokes);
            }
        } else {
            match sp.finish {
                Finish::Carved => recessed(fills, &piece, sp.tiles[t.kind], mix(sp.tiles[t.kind], m.dark, 0.85)),
                Finish::Glazed => glazed(fills, &piece, sp.tiles[t.kind]),
            }
            // The inlay is scaled from the whole tile, so where the frame cuts the tile it cuts the inlay too.
            let small = clip(&scale(&inner, 0.55), &piece);
            if let (Some(inlay), true) = (sp.inlay, area(&small) > 2e-4) {
                match sp.finish {
                    Finish::Carved => raised(fills, &small, &piece, inlay[t.kind], m.dark),
                    Finish::Glazed => {
                        glazed(fills, &small, inlay[t.kind]);
                        edge(small, strokes);
                    }
                }
            }
        }
        edge(piece, strokes);
    }
}

/// The tile at the centre, made a focal point: raised and recessed in carving, a metal boss with an
/// inlay in tilework, and a set gem at the top.
fn centrepiece(sp: &Spec, piece: &[Point], bound: &[Point], fills: &mut Vec<Fill>) {
    let m = sp.palette.metal;
    if sp.centre == Centre::Raised {
        raised(fills, piece, bound, m.light, m.dark);
        recessed(fills, &scale(piece, 0.5), m.mid, mix(m.mid, m.dark, 0.7));
        return;
    }
    fills.push(Fill { points: piece.to_vec(), color: m.dark });
    fills.push(Fill { points: clip(&shift(&scale(piece, 0.93), (-0.006, 0.006)), piece), color: m.mid });
    let inlay = scale(piece, 0.62);
    glazed(fills, &inlay, sp.jewel);
    if sp.centre == Centre::Jewelled {
        gem(fills, (0.0, 0.0), 0.05, sp.tiles[1], m);
    }
}

/// A panel sunk below the strapwork: lit from the top left, its top-left wall falls in shadow.
fn recessed(fills: &mut Vec<Fill>, piece: &[Point], floor: Rgb, wall: Rgb) {
    fills.push(Fill { points: piece.to_vec(), color: wall });
    fills.push(Fill { points: clip(&shift(&scale(piece, 0.9), (0.016, -0.016)), piece), color: floor });
}

/// A boss standing proud of its surround: its shadow falls to the lower right, kept within `within`.
fn raised(fills: &mut Vec<Fill>, piece: &[Point], within: &[Point], face: Rgb, shadow: Rgb) {
    fills.push(Fill { points: clip(&shift(piece, (0.01, -0.01)), within), color: shadow });
    fills.push(Fill { points: piece.to_vec(), color: face });
}

/// A flush glazed tile with a soft highlight towards the light.
fn glazed(fills: &mut Vec<Fill>, piece: &[Point], color: Rgb) {
    fills.push(Fill { points: piece.to_vec(), color });
    fills.push(Fill { points: clip(&shift(&scale(piece, 0.8), (-0.008, 0.008)), piece), color: mix(color, (0xff, 0xff, 0xff), 0.14) });
}

/// A round bead or pearl: a shadow to the lower right, the bead, and a highlight to the upper left.
fn bead(fills: &mut Vec<Fill>, c: Point, r: f64, m: Metal) {
    fills.push(Fill { points: circle((c.0 + 0.22 * r, c.1 - 0.22 * r), r, 16), color: m.dark });
    fills.push(Fill { points: circle(c, r, 16), color: m.mid });
    fills.push(Fill { points: circle((c.0 - 0.32 * r, c.1 + 0.32 * r), 0.36 * r, 12), color: m.light });
}

/// A cabochon gem in a metal bezel, with a highlight.
fn gem(fills: &mut Vec<Fill>, c: Point, r: f64, color: Rgb, m: Metal) {
    fills.push(Fill { points: circle(c, 1.32 * r, 24), color: m.dark });
    fills.push(Fill { points: circle(c, 1.2 * r, 24), color: m.mid });
    let stone = circle(c, r, 24);
    fills.push(Fill { points: stone.clone(), color: mix(color, (0, 0, 0), 0.35) });
    fills.push(Fill { points: clip(&shift(&scale(&stone, 0.86), (-0.12 * r, 0.12 * r)), &stone), color });
    fills.push(Fill { points: circle((c.0 - 0.36 * r, c.1 + 0.36 * r), 0.26 * r, 12), color: mix(color, (0xff, 0xff, 0xff), 0.8) });
}

/// The border band between the field and the rim, and its ornament.
fn border(sp: &Spec, fills: &mut Vec<Fill>, strokes: &mut Vec<Stroke>) {
    let m = sp.palette.metal;
    let mid = (FIELD + BAND) / 2.0;
    fills.push(Fill { points: circle((0.0, 0.0), BAND, CIRCLE), color: m.mid });
    let fillet = |r: f64, strokes: &mut Vec<Stroke>| {
        let ring = circle((0.0, 0.0), r, CIRCLE);
        strokes.push(Stroke { points: shift(&ring, (0.005, -0.005)), role: Role::Band, color: m.dark, width: 0.018 });
        strokes.push(Stroke { points: shift(&ring, (-0.003, 0.003)), role: Role::Band, color: m.light, width: 0.009 });
    };
    let around = |n: usize, turn: f64| (0..n).map(move |i| PI / 2.0 + turn + 2.0 * PI * i as f64 / n as f64);
    match sp.border {
        Border::Plain => {}
        Border::Fillet => fillet(mid, strokes),
        Border::Beads => {
            for a in around(48, 0.0) {
                bead(fills, polar(mid, a), 0.024, m);
            }
        }
        Border::Rope => {
            // A twisted cord: slanted ovals overlapping round the band.
            for a in around(56, 0.0) {
                let (n, t) = ((a.cos(), a.sin()), (-a.sin(), a.cos()));
                let slant = 0.75_f64;
                let u = (n.0 * slant.sin() + t.0 * slant.cos(), n.1 * slant.sin() + t.1 * slant.cos());
                let w = (-u.1, u.0);
                let c = polar(mid, a);
                let oval = ccw((0..14).map(|k| {
                    let f = 2.0 * PI * k as f64 / 14.0;
                    (c.0 + u.0 * 0.056 * f.cos() + w.0 * 0.02 * f.sin(), c.1 + u.1 * 0.056 * f.cos() + w.1 * 0.02 * f.sin())
                }).collect());
                fills.push(Fill { points: shift(&oval, (0.005, -0.005)), color: m.dark });
                fills.push(Fill { points: oval.clone(), color: mix(m.mid, m.light, 0.35) });
                fills.push(Fill { points: shift(&scale(&oval, 0.45), (-0.006, 0.006)), color: m.light });
            }
        }
        Border::BeadsAndFillet => {
            fillet(BAND - 0.022, strokes);
            for a in around(36, 0.0) {
                bead(fills, polar(mid - 0.012, a), 0.02, m);
            }
        }
        Border::Jewels | Border::Crown => {
            for (i, a) in around(24, 0.0).enumerate() {
                if sp.border == Border::Crown {
                    gem(fills, polar(mid, a), 0.026, if i % 2 == 0 { TURQUOISE } else { LAPIS }, m);
                } else {
                    let (n, t) = ((a.cos(), a.sin()), (-a.sin(), a.cos()));
                    let c = polar(mid, a);
                    let lozenge = ccw(vec![(c.0 + t.0 * 0.05, c.1 + t.1 * 0.05), (c.0 + n.0 * 0.038, c.1 + n.1 * 0.038), (c.0 - t.0 * 0.05, c.1 - t.1 * 0.05), (c.0 - n.0 * 0.038, c.1 - n.1 * 0.038)]);
                    fills.push(Fill { points: shift(&lozenge, (0.005, -0.005)), color: m.dark });
                    fills.push(Fill { points: lozenge.clone(), color: m.mid });
                    glazed(fills, &scale(&lozenge, 0.5), sp.tiles[0]);
                }
            }
            for a in around(24, PI / 24.0) {
                bead(fills, polar(mid, a), 0.016, m);
            }
            if sp.border == Border::Crown {
                for a in around(60, 0.0) {
                    bead(fills, polar((BAND + 1.0) / 2.0, a), 0.0115, PEARL);
                }
            }
        }
    }
}

/// A plain n-gon, counter-clockwise, its first vertex at angle `rotation`.
fn regular_polygon(n: usize, radius: f64, rotation: f64) -> Vec<Point> {
    (0..n).map(|i| polar(radius, rotation + 2.0 * PI * i as f64 / n as f64)).collect()
}

fn circle(c: Point, r: f64, n: usize) -> Vec<Point> {
    shift(&regular_polygon(n, r, 0.0), c)
}

fn shift(poly: &[Point], d: Point) -> Vec<Point> {
    poly.iter().map(|p| (p.0 + d.0, p.1 + d.1)).collect()
}

/// The polygon scaled by `k` about its centroid.
fn scale(poly: &[Point], k: f64) -> Vec<Point> {
    let c = centroid(poly);
    poly.iter().map(|p| (c.0 + (p.0 - c.0) * k, c.1 + (p.1 - c.1) * k)).collect()
}

/// Twice nothing: the signed area, positive when counter-clockwise.
fn area(poly: &[Point]) -> f64 {
    let n = poly.len();
    (0..n).map(|i| poly[i].0 * poly[(i + 1) % n].1 - poly[(i + 1) % n].0 * poly[i].1).sum::<f64>() / 2.0
}

fn centroid(poly: &[Point]) -> Point {
    let (a, n) = (area(poly), poly.len());
    if a.abs() < 1e-12 {
        let k = n.max(1) as f64;
        return poly.iter().fold((0.0, 0.0), |s, p| (s.0 + p.0 / k, s.1 + p.1 / k));
    }
    let (mut x, mut y) = (0.0, 0.0);
    for i in 0..n {
        let (p, q) = (poly[i], poly[(i + 1) % n]);
        let w = p.0 * q.1 - q.0 * p.1;
        x += (p.0 + q.0) * w;
        y += (p.1 + q.1) * w;
    }
    (x / (6.0 * a), y / (6.0 * a))
}

fn ccw(mut poly: Vec<Point>) -> Vec<Point> {
    if area(&poly) < 0.0 {
        poly.reverse();
    }
    poly
}

fn dist2(a: Point, b: Point) -> f64 {
    (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)
}

/// A convex counter-clockwise polygon with every edge moved inward by `d`, or None if it vanishes.
fn inset(poly: &[Point], d: f64) -> Option<Vec<Point>> {
    let n = poly.len();
    let line = |i: usize| {
        let (p, q) = (poly[i], poly[(i + 1) % n]);
        let (dx, dy) = (q.0 - p.0, q.1 - p.1);
        let len = dx.hypot(dy);
        ((p.0 - dy / len * d, p.1 + dx / len * d), (dx, dy))
    };
    let out: Vec<Point> = (0..n)
        .map(|i| {
            let ((p, u), (q, v)) = (line((i + n - 1) % n), line(i));
            intersect(p, u, q, v)
        })
        .collect::<Option<_>>()?;
    (area(&out) > 0.0 && area(&out) < area(poly)).then_some(out)
}

/// The part of `subject` inside the convex counter-clockwise polygon `bound` (Sutherland–Hodgman).
fn clip(subject: &[Point], bound: &[Point]) -> Vec<Point> {
    let mut out = subject.to_vec();
    for i in 0..bound.len() {
        if out.is_empty() {
            break;
        }
        let (a, b) = (bound[i], bound[(i + 1) % bound.len()]);
        let inside = |p: Point| (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0) >= 0.0;
        let cut = |p: Point, q: Point| intersect(p, (q.0 - p.0, q.1 - p.1), a, (b.0 - a.0, b.1 - a.1)).unwrap_or(q);
        let input = std::mem::take(&mut out);
        for j in 0..input.len() {
            let (p, q) = (input[j], input[(j + 1) % input.len()]);
            match (inside(p), inside(q)) {
                (true, true) => out.push(q),
                (true, false) => out.push(cut(p, q)),
                (false, true) => {
                    out.push(cut(p, q));
                    out.push(q);
                }
                (false, false) => {}
            }
        }
    }
    out
}

/// `a` moved towards `b` by `t`.
pub(crate) fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let m = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

fn polar(r: f64, a: f64) -> Point {
    (r * a.cos(), r * a.sin())
}

pub(crate) fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

/// A coordinate to two places, with no negative zero so equal shapes print equally.
pub(crate) fn num(v: f64) -> String {
    let s = format!("{v:.2}");
    if s == "-0.00" { "0.00".into() } else { s }
}

/// An SVG path's `d` for a closed polygon mapped to the centre (cx, cy) and radius r, y up.
pub(crate) fn path(points: &[Point], cx: f64, cy: f64, r: f64) -> String {
    let mut d = String::new();
    for (i, p) in points.iter().enumerate() {
        let _ = write!(d, "{}{} {}", if i == 0 { "M" } else { " L" }, num(cx + p.0 * r), num(cy - p.1 * r));
    }
    d.push_str(" Z");
    d
}

/// A tier's badge as an SVG `<g>`, centred on (cx, cy) with radius r: the bevelled rim, the fills and
/// strokes in order, then a vignette over the field and a sheen over the whole.
pub fn svg(tier: usize, cx: f64, cy: f64, r: f64) -> String {
    let b = badge(tier);
    let m = b.palette.metal;
    let id = format!("badge{}", tier.min(TIERS - 1));
    let mut out = String::from("<g>");
    let _ = write!(out, "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"url(#{id}-rim)\"/>", num(cx), num(cy), num(r));
    let _ = write!(
        out,
        "<defs><linearGradient id=\"{id}-rim\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"1\"><stop offset=\"0\" stop-color=\"{l}\"/><stop offset=\"0.5\" stop-color=\"{m}\"/><stop offset=\"1\" stop-color=\"{d}\"/></linearGradient>\
         <linearGradient id=\"{id}-bevel\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"1\"><stop offset=\"0\" stop-color=\"{d}\"/><stop offset=\"0.5\" stop-color=\"{m}\"/><stop offset=\"1\" stop-color=\"{l}\"/></linearGradient>\
         <radialGradient id=\"{id}-vignette\" cx=\"0.5\" cy=\"0.5\" r=\"0.5\"><stop offset=\"0.6\" stop-color=\"#000\" stop-opacity=\"0\"/><stop offset=\"1\" stop-color=\"#000\" stop-opacity=\"0.3\"/></radialGradient>\
         <linearGradient id=\"{id}-sheen\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"1\"><stop offset=\"0\" stop-color=\"#fff\" stop-opacity=\"{g}\"/><stop offset=\"0.55\" stop-color=\"#fff\" stop-opacity=\"0\"/></linearGradient></defs>",
        l = hex(m.light),
        m = hex(m.mid),
        d = hex(m.dark),
        g = num(b.gloss),
    );
    let _ = write!(out, "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"url(#{id}-bevel)\"/>", num(cx), num(cy), num(0.96 * r));
    for f in &b.fills {
        let _ = write!(out, "<path d=\"{}\" fill=\"{}\"/>", path(&f.points, cx, cy, r), hex(f.color));
    }
    out.push_str("<g fill=\"none\" stroke-linejoin=\"round\" stroke-linecap=\"round\">");
    for s in &b.strokes {
        let _ = write!(out, "<path d=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>", path(&s.points, cx, cy, r), hex(s.color), num(s.width * r));
    }
    out.push_str("</g>");
    let _ = write!(out, "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"url(#{id}-vignette)\"/>", num(cx), num(cy), num(FIELD * r));
    let _ = write!(out, "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"url(#{id}-sheen)\"/>", num(cx), num(cy), num(r));
    out.push_str("</g>");
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
            for (a, b, _) in badge(tier).segments() {
                assert!(dist(a) <= 1.0 + EPS && dist(b) <= 1.0 + EPS, "tier {tier}");
            }
        }
    }

    #[test]
    fn every_fill_is_convex_so_no_shape_is_a_star() {
        for tier in 0..TIERS {
            for f in &badge(tier).fills {
                let n = f.points.len();
                assert!(n >= 3, "tier {tier} has a degenerate fill");
                for i in 0..n {
                    let (a, b, c) = (f.points[i], f.points[(i + 1) % n], f.points[(i + 2) % n]);
                    let turn = (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0);
                    assert!(turn >= -1e-9, "tier {tier} has a concave fill at {b:?}");
                }
            }
        }
    }

    #[test]
    fn carved_tiers_are_one_warm_material_and_tiled_tiers_carry_blue_glaze() {
        for tier in [0, 1] {
            for f in &badge(tier).fills {
                let (r, g, b) = f.color;
                assert!(r >= g && g >= b, "tier {tier} fill {:?} is not the carved material", f.color);
            }
        }
        for tier in 2..TIERS {
            assert!(badge(tier).fills.iter().any(|f| f.color.2 as i32 > f.color.0 as i32 + 40), "tier {tier} has no blue glaze");
        }
    }

    #[test]
    fn each_rank_adds_detail() {
        let detail: Vec<usize> = (0..TIERS).map(|t| badge(t).fills.len() + badge(t).strokes.len()).collect();
        for w in detail.windows(2) {
            assert!(w[0] < w[1], "detail by tier {detail:?}");
        }
    }

    #[test]
    fn tile_patterns_keep_their_rotational_symmetry() {
        for (tier, fold) in [4, 6, 4, 4, 4, 10, 10].into_iter().enumerate() {
            let centres: Vec<Point> = badge(tier).strokes.iter().filter(|s| s.role == Role::Tile).map(|s| centroid(&s.points)).collect();
            assert!(centres.len() > fold, "tier {tier} has too few tiles");
            for &c in &centres {
                let turned = rotate(c, 2.0 * PI / fold as f64);
                assert!(centres.iter().any(|&d| dist2(d, turned) < 1e-10), "tier {tier}: tile at {c:?} has no partner a 1/{fold} turn away");
            }
        }
    }

    #[test]
    fn decagon_rings_close_into_a_bigger_decagon() {
        // The rhombus tips of each ring are the corners of a regular decagon twice the side.
        let tiles = tiles(Pattern::Decagons(0.86));
        let tips: Vec<f64> = tiles.iter().filter(|t| t.kind == 2).map(|t| t.points.iter().map(|&p| dist(p)).fold(0.0, f64::max)).collect();
        assert_eq!(tips.len(), 20);
        assert!((tips[10] - 0.86).abs() < 1e-9, "outer ring reaches {}", tips[10]);
        assert!((tips[0] - 0.43).abs() < 1e-9, "inner ring reaches {}", tips[0]);
    }

    #[test]
    fn inset_and_clip_keep_shapes_inside() {
        let square = regular_polygon(4, 1.0, PI / 4.0);
        let small = inset(&square, 0.1).unwrap();
        assert!((area(&small) - (2f64.sqrt() - 0.2).powi(2)).abs() < 1e-9);
        assert!(inset(&square, 5.0).is_none(), "an inset past the middle vanishes");
        let half = clip(&square, &[(0.0, -5.0), (5.0, -5.0), (5.0, 5.0), (0.0, 5.0)]);
        assert!((area(&half) - area(&square) / 2.0).abs() < 1e-9);
    }

    #[test]
    #[ignore = "writes a preview SVG of all tiers to $BADGE_PREVIEW"]
    fn preview_all_tiers() {
        let path = std::env::var("BADGE_PREVIEW").expect("set BADGE_PREVIEW");
        std::fs::write(path, preview()).unwrap();
    }

    #[test]
    #[ignore = "writes a preview PNG of all tiers to $BADGE_PREVIEW_PNG"]
    fn preview_all_tiers_png() {
        let path = std::env::var("BADGE_PREVIEW_PNG").expect("set BADGE_PREVIEW_PNG");
        let tree = resvg::usvg::Tree::from_str(&preview(), &resvg::usvg::Options::default()).unwrap();
        let mut pixmap = resvg::tiny_skia::Pixmap::new(1680, 260).unwrap();
        resvg::render(&tree, resvg::tiny_skia::Transform::default(), &mut pixmap.as_mut());
        pixmap.save_png(path).unwrap();
    }

    fn preview() -> String {
        let mut out = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1680\" height=\"260\" viewBox=\"0 0 1680 260\"><rect width=\"1680\" height=\"260\" fill=\"#f4f1ea\"/>");
        for tier in 0..TIERS {
            out.push_str(&svg(tier, 120.0 + 240.0 * tier as f64, 130.0, 110.0));
        }
        out.push_str("</svg>");
        out
    }
}
