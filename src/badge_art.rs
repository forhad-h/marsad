//! Badge art: a deterministic Islamic geometric roundel per tier, after three historical traditions.
//! Novice and Apprentice are Mamluk / Seljuk carved roundels, one material read through light and shadow; Practitioner to Expert are Moorish zellij, cut tiles under woven white bands; Master and Grandmaster are Persian girih, ten-fold strapwork in turquoise, lapis and gold.
//! Every pattern is Hankin's "polygons in contact" method on a rosette tiling, clipped to a disc.

use std::collections::HashMap;
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
    let (s, _) = params(p, u, q, v)?;
    Some(add(p, mul(u, s)))
}

/// The s and t at which p + s·u meets q + t·v, if they are not parallel.
fn params(p: Point, u: Point, q: Point, v: Point) -> Option<(f64, f64)> {
    let den = cross(u, v);
    if den.abs() < 1e-12 {
        return None;
    }
    let w = sub(q, p);
    Some((cross(w, v) / den, cross(w, u) / den))
}

/// Where two rays meet ahead of both starts.
fn meet(p: Point, u: Point, q: Point, v: Point) -> Option<Point> {
    let (s, t) = params(p, u, q, v)?;
    (s > 1e-9 && t > 1e-9).then(|| add(p, mul(u, s)))
}

fn add(a: Point, b: Point) -> Point {
    (a.0 + b.0, a.1 + b.1)
}

fn sub(a: Point, b: Point) -> Point {
    (a.0 - b.0, a.1 - b.1)
}

fn mul(a: Point, k: f64) -> Point {
    (a.0 * k, a.1 * k)
}

fn dot(a: Point, b: Point) -> f64 {
    a.0 * b.0 + a.1 * b.1
}

fn cross(a: Point, b: Point) -> f64 {
    a.0 * b.1 - a.1 * b.0
}

fn norm(a: Point) -> f64 {
    a.0.hypot(a.1)
}

fn unit(a: Point) -> Point {
    mul(a, 1.0 / norm(a))
}

fn mid(a: Point, b: Point) -> Point {
    lerp(a, b, 0.5)
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

fn polar(r: f64, a: f64) -> Point {
    (r * a.cos(), r * a.sin())
}

fn close(a: Point, b: Point) -> bool {
    norm(sub(a, b)) < 1e-7
}

/// The historical tradition a tier's badge is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Mamluk / Seljuk carved roundel: raised strapwork with a groove, recessed panels, one material.
    Carved,
    /// Moorish zellij: cut-tile pieces in a bright palette under white bands outlined in black.
    Zellij,
    /// Persian girih: ten-fold strapwork in gold over turquoise and lapis tiles.
    Girih,
}

/// What a stroke is part of; the badge view colours by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A border ring around the pattern.
    Ring,
    /// An interlaced band of the pattern.
    Strap,
}

/// A polyline in the unit space.
#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub points: Vec<Point>,
    pub closed: bool,
    pub role: Role,
}

/// A filled polygon in the unit space: one tile or panel between the bands.
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    pub points: Vec<Point>,
    pub color: Rgb,
}

/// A tier's colours. The card takes its accents from them too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// The darkest tone: the disc behind the pattern, and the card's frame band.
    pub ground: Rgb,
    /// The main mid tone: the card's accent text and lattice.
    pub field: Rgb,
    /// A light secondary tone: a carved badge's panel floor, a tiled badge's accent tile.
    pub rosette: Rgb,
    /// The band's outline, or a carved badge's shadow.
    pub band: Rgb,
    /// The band's face.
    pub line: Rgb,
}

/// How a tier's badge is built.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Recipe {
    pub style: Style,
    /// The rosette's fold: the centre polygon's side count.
    pub fold: usize,
    /// The angle, in degrees, at which the bands cross every tile edge.
    pub contact: f64,
    /// Rings of tiles around the centre polygon: 1 adds a pentagon on each edge, 2 an octagon in each notch between them.
    pub rings: usize,
    /// The centre polygon's circumradius; a smaller one shows more of the pattern.
    pub scale: f64,
    pub palette: Palette,
    /// Tile colours, given out to the pieces ring by ring from the centre.
    pub tiles: &'static [Rgb],
}

const IVORY: Palette = Palette { ground: (0x6b, 0x52, 0x34), field: (0x8a, 0x6a, 0x40), rosette: (0xbd, 0xa3, 0x78), band: (0x4a, 0x36, 0x20), line: (0xee, 0xe2, 0xc6) };
const CEDAR: Palette = Palette { ground: (0x3e, 0x22, 0x14), field: (0x7a, 0x44, 0x24), rosette: (0x8c, 0x52, 0x2e), band: (0x24, 0x12, 0x0a), line: (0xc4, 0x86, 0x52) };
const FEZ: Palette = Palette { ground: (0x16, 0x2a, 0x4e), field: (0x1d, 0x4f, 0x91), rosette: (0x2e, 0x9e, 0x9a), band: (0x15, 0x15, 0x18), line: (0xf6, 0xf2, 0xe8) };
const ISFAHAN: Palette = Palette { ground: (0x10, 0x22, 0x5a), field: (0x1c, 0x4a, 0x9a), rosette: (0x2a, 0xa8, 0xb4), band: (0x0a, 0x14, 0x38), line: (0xe0, 0xb4, 0x4a) };
const TIMURID: Palette = Palette { ground: (0x0e, 0x1c, 0x4a), field: (0x9a, 0x6e, 0x1c), rosette: (0x2a, 0xa8, 0xb4), band: (0x0a, 0x12, 0x30), line: (0xf0, 0xca, 0x60) };

const COBALT: Rgb = (0x1d, 0x4f, 0x91);
const TURQUOISE: Rgb = (0x2e, 0x9e, 0x9a);
const GREEN: Rgb = (0x2f, 0x7a, 0x4a);
const OCHRE: Rgb = (0xd0, 0x9a, 0x2c);
const BLACK: Rgb = (0x1e, 0x1e, 0x22);
const WHITE: Rgb = (0xf4, 0xf0, 0xe4);
const LAPIS: Rgb = (0x1c, 0x3a, 0x8a);
const DEEP_LAPIS: Rgb = (0x12, 0x26, 0x66);
const TEAL: Rgb = (0x2a, 0xa8, 0xb4);
const GOLD: Rgb = (0xd8, 0xa8, 0x40);
const CREAM: Rgb = (0xf2, 0xe8, 0xd0);

/// The recipe for a tier; a tier past the top is Grandmaster.
pub fn recipe(tier: usize) -> Recipe {
    let r = |style, fold, contact, rings, scale, palette, tiles| Recipe { style, fold, contact, rings, scale, palette, tiles };
    match tier.min(TIERS - 1) {
        0 => r(Style::Carved, 8, 45.0, 1, 0.40, IVORY, &[]),
        1 => r(Style::Carved, 12, 67.5, 1, 0.42, CEDAR, &[]),
        2 => r(Style::Zellij, 8, 67.5, 1, 0.36, FEZ, &[OCHRE, COBALT, GREEN, BLACK, TURQUOISE, WHITE]),
        3 => r(Style::Zellij, 8, 67.5, 2, 0.28, FEZ, &[WHITE, COBALT, OCHRE, BLACK, TURQUOISE, COBALT, GREEN]),
        4 => r(Style::Zellij, 8, 67.5, 2, 0.24, FEZ, &[OCHRE, COBALT, GREEN, BLACK, TURQUOISE, WHITE, COBALT]),
        5 => r(Style::Girih, 10, 54.0, 1, 0.34, ISFAHAN, &[TEAL, DEEP_LAPIS, CREAM, LAPIS, TEAL, DEEP_LAPIS]),
        _ => r(Style::Girih, 10, 54.0, 2, 0.24, TIMURID, &[GOLD, DEEP_LAPIS, CREAM, LAPIS, TEAL, DEEP_LAPIS, TEAL, CREAM, LAPIS]),
    }
}

/// The pattern's edge: the bands are clipped here and the border ring runs outside it.
const RIM: f64 = 0.84;
/// How far the boundary rays run before clipping, past the rim.
const REACH: f64 = 1.2;
/// Segments in a ring's polyline.
const RING_SEGMENTS: usize = 192;

fn regular(n: usize, centre: Point, r: f64, rotation: f64) -> Vec<Point> {
    (0..n).map(|i| add(centre, polar(r, rotation + 2.0 * PI * i as f64 / n as f64))).collect()
}

/// The rosette tiling, every polygon counter-clockwise: a regular n-gon with a vertex at the top, with one ring a pentagon on each of its edges, and with two (n = 8 only, where the notch is 135°) a regular octagon in each notch between the pentagons.
fn tiling(n: usize, rings: usize, scale: f64) -> Vec<Vec<Point>> {
    let up = PI / 2.0;
    let step = 2.0 * PI / n as f64;
    let centre = regular(n, (0.0, 0.0), scale, up);
    let edge = norm(sub(centre[1], centre[0]));
    let mut tiles = vec![centre.clone()];
    if rings == 0 {
        return tiles;
    }
    // The pentagon's angle at its shoulders: 108° makes it regular for n = 10, and 112.5° leaves notches of 135° for n = 8.
    let shoulder_angle = match n {
        8 => 112.5_f64,
        10 => 108.0,
        _ => 120.0,
    }
    .to_radians();
    let shoulder = |i: usize| polar(scale + edge, up + step * i as f64);
    let apex = |i: usize| {
        let a = up + step * i as f64;
        intersect(shoulder(i), polar(1.0, a + PI - shoulder_angle), (0.0, 0.0), polar(1.0, a + step / 2.0)).expect("apex on the bisector")
    };
    for i in 0..n {
        tiles.push(vec![centre[i], shoulder(i), apex(i), shoulder(i + 1), centre[(i + 1) % n]]);
    }
    if rings < 2 {
        return tiles;
    }
    let side = norm(sub(apex(0), shoulder(0)));
    for i in 0..n {
        let a = up + step * i as f64;
        if n == 8 {
            let r = side / (2.0 * (PI / 8.0).sin());
            tiles.push(regular(8, add(shoulder(i), polar(r, a)), r, a + PI));
        } else {
            // Elsewhere a hexagon fills the notch: out from both apexes along their bisectors, closing at a point on the notch's axis.
            let (p, q) = (apex(i + n - 1), apex(i));
            let (p2, q2) = (add(p, mul(unit(p), side)), add(q, mul(unit(q), side)));
            let tip = polar(norm(p2) / (step / 2.0).cos() + side * 0.35, a);
            tiles.push(vec![q, shoulder(i), p, p2, tip, q2]);
        }
    }
    tiles
}

/// Hankin's rays inside one convex polygon: from each edge midpoint, a ray at `contact` towards each end, stopped where it meets the neighbouring edge's ray.
fn hankin_tile(poly: &[Point], contact: f64, out: &mut Vec<(Point, Point)>) {
    let n = poly.len();
    let (c, s) = (contact.cos(), contact.sin());
    let edge = |j: usize| {
        let (a, b) = (poly[j % n], poly[(j + 1) % n]);
        let e = unit(sub(b, a));
        (mid(a, b), e, (-e.1, e.0))
    };
    for j in 0..n {
        let (m, e, inward) = edge(j);
        let (m2, e2, inward2) = edge(j + 1);
        let u = add(mul(e, c), mul(inward, s));
        let w = add(mul(e2, -c), mul(inward2, s));
        if let Some(x) = meet(m, u, m2, w) {
            out.push((m, x));
            out.push((x, m2));
        }
    }
}

/// The pattern's segments: Hankin's rays in every tile, then rays outward from the patch's boundary edges, which meet across its notches or run on to `reach`.
fn pattern(tiles: &[Vec<Point>], contact: f64, reach: f64) -> Vec<(Point, Point)> {
    let mut segs = Vec::new();
    for t in tiles {
        hankin_tile(t, contact, &mut segs);
    }
    let edges: Vec<(Point, Point)> = tiles.iter().flat_map(|t| (0..t.len()).map(move |j| (t[j], t[(j + 1) % t.len()]))).collect();
    let boundary: Vec<(Point, Point)> = edges.iter().copied().filter(|&(a, b)| !edges.iter().any(|&(p, q)| close(p, b) && close(q, a))).collect();
    let (c, s) = (contact.cos(), contact.sin());
    // The outward rays of a boundary edge: towards its start and towards its end.
    let rays = |(a, b): (Point, Point)| {
        let e = unit(sub(b, a));
        let out = (e.1, -e.0);
        (mid(a, b), add(mul(e, -c), mul(out, s)), add(mul(e, c), mul(out, s)))
    };
    let run = |m: Point, u: Point| {
        let b = dot(m, u);
        let t = -b + (b * b - dot(m, m) + reach * reach).max(0.0).sqrt();
        add(m, mul(u, t))
    };
    for &(a, b) in &boundary {
        let (m, to_a, to_b) = rays((a, b));
        let next = boundary.iter().find(|e| close(e.0, b)).map(|&e| rays(e));
        let prev = boundary.iter().find(|e| close(e.1, a)).map(|&e| rays(e));
        let hit_b = next.and_then(|(m2, back, _)| meet(m, to_b, m2, back)).filter(|&x| norm(x) < reach);
        let hit_a = prev.and_then(|(m2, _, fwd)| meet(m, to_a, m2, fwd)).filter(|&x| norm(x) < reach);
        segs.push((m, hit_b.unwrap_or_else(|| run(m, to_b))));
        segs.push((m, hit_a.unwrap_or_else(|| run(m, to_a))));
    }
    segs
}

/// Joins segments into strands: through a point where two segments meet, and straight across a point where four do.
/// Returns each strand's points and whether it closes on itself.
fn chain(segs: &[(Point, Point)]) -> Vec<(Vec<Point>, bool)> {
    // Every segment end, grouped by where it lies.
    let mut groups: Vec<(Point, Vec<(usize, usize)>)> = Vec::new();
    for (i, s) in segs.iter().enumerate() {
        for (k, p) in [s.0, s.1].into_iter().enumerate() {
            match groups.iter_mut().find(|g| close(g.0, p)) {
                Some(g) => g.1.push((i, k)),
                None => groups.push((p, vec![(i, k)])),
            }
        }
    }
    let end = |(i, k): (usize, usize)| if k == 0 { segs[i].0 } else { segs[i].1 };
    let away = |(i, k): (usize, usize)| if k == 0 { unit(sub(segs[i].1, segs[i].0)) } else { unit(sub(segs[i].0, segs[i].1)) };
    // Each end's partner at the same point, if any.
    let mut partner: HashMap<(usize, usize), (usize, usize)> = HashMap::new();
    for (_, ends) in &groups {
        let mut left = ends.clone();
        while left.len() >= 2 && left.len() % 2 == 0 {
            let e = left.remove(0);
            let (j, _) = left.iter().enumerate().min_by(|a, b| dot(away(e), away(*a.1)).total_cmp(&dot(away(e), away(*b.1)))).expect("an end left");
            let f = left.remove(j);
            partner.insert(e, f);
            partner.insert(f, e);
        }
    }
    let mut used = vec![false; segs.len()];
    let mut strands = Vec::new();
    // Open strands start at an unpartnered end; whatever remains is a closed loop.
    let starts: Vec<(usize, usize)> = (0..segs.len()).flat_map(|i| [(i, 0), (i, 1)]).filter(|e| !partner.contains_key(e)).chain((0..segs.len()).map(|i| (i, 0))).collect();
    for start in starts {
        if used[start.0] {
            continue;
        }
        let mut points = vec![end(start)];
        let mut at = start;
        let closed = loop {
            used[at.0] = true;
            let other = (at.0, 1 - at.1);
            points.push(end(other));
            match partner.get(&other) {
                Some(&next) if !used[next.0] => at = next,
                Some(&next) if next == start => break true,
                _ => break false,
            }
        };
        if closed {
            points.pop();
        }
        strands.push((straighten(points, closed), closed));
    }
    strands
}

/// Drops the points a polyline passes straight through.
fn straighten(points: Vec<Point>, closed: bool) -> Vec<Point> {
    let n = points.len();
    let keep = |i: usize| {
        if !closed && (i == 0 || i == n - 1) {
            return true;
        }
        let (a, b, c) = (points[(i + n - 1) % n], points[i], points[(i + 1) % n]);
        let (u, v) = (sub(b, a), sub(c, b));
        cross(u, v).abs() > 1e-9 * norm(u) * norm(v) || dot(u, v) < 0.0
    };
    (0..n).filter(|&i| keep(i)).map(|i| points[i]).collect()
}

/// Clips strands to the disc of radius `r`, splitting any that leave and re-enter it.
fn clip(strands: Vec<(Vec<Point>, bool)>, r: f64) -> Vec<(Vec<Point>, bool)> {
    let inside = |p: Point| norm(p) < r;
    let mut out = Vec::new();
    for (mut points, closed) in strands {
        if points.iter().all(|&p| inside(p)) {
            out.push((points, closed));
            continue;
        }
        if closed {
            // Start a loop outside, so every piece inside is whole.
            let k = points.iter().position(|&p| !inside(p)).expect("a point outside");
            points.rotate_left(k);
            points.push(points[0]);
        }
        let mut piece: Vec<Point> = Vec::new();
        for w in points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let d = sub(b, a);
            // |a + t·d| = r
            let (qa, qb, qc) = (dot(d, d), 2.0 * dot(a, d), dot(a, a) - r * r);
            let disc = qb * qb - 4.0 * qa * qc;
            let mut cuts: Vec<f64> = if disc > 0.0 { [(-qb - disc.sqrt()) / (2.0 * qa), (-qb + disc.sqrt()) / (2.0 * qa)].into_iter().filter(|t| *t > 0.0 && *t < 1.0).collect() } else { vec![] };
            cuts.insert(0, 0.0);
            cuts.push(1.0);
            for c in cuts.windows(2) {
                let (p, q) = (lerp(a, b, c[0]), lerp(a, b, c[1]));
                if inside(lerp(a, b, (c[0] + c[1]) / 2.0)) {
                    if piece.is_empty() {
                        piece.push(p);
                    }
                    piece.push(q);
                } else if piece.len() >= 2 {
                    out.push((std::mem::take(&mut piece), false));
                } else {
                    piece.clear();
                }
            }
        }
        if piece.len() >= 2 {
            out.push((piece, false));
        }
    }
    out
}

/// Splits segments wherever they cross or touch, and returns the planar graph: vertices and, for each, its neighbours.
fn arrange(segs: &[(Point, Point)]) -> (Vec<Point>, Vec<Vec<usize>>) {
    let mut cuts: Vec<Vec<f64>> = vec![vec![0.0, 1.0]; segs.len()];
    for i in 0..segs.len() {
        for j in i + 1..segs.len() {
            let ((a, b), (c, d)) = (segs[i], segs[j]);
            if let Some((t, u)) = params(a, sub(b, a), c, sub(d, c)) {
                let e = 1e-9;
                if (-e..=1.0 + e).contains(&t) && (-e..=1.0 + e).contains(&u) {
                    cuts[i].push(t.clamp(0.0, 1.0));
                    cuts[j].push(u.clamp(0.0, 1.0));
                }
            }
        }
    }
    let mut verts: Vec<Point> = Vec::new();
    let mut index: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let mut vertex = |p: Point, verts: &mut Vec<Point>| {
        let key = ((p.0 * 1e6).round() as i64, (p.1 * 1e6).round() as i64);
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(ids) = index.get(&(key.0 + dx, key.1 + dy))
                    && let Some(&v) = ids.iter().find(|&&v| close(verts[v], p))
                {
                    return v;
                }
            }
        }
        verts.push(p);
        index.entry(key).or_default().push(verts.len() - 1);
        verts.len() - 1
    };
    let mut adj: Vec<Vec<usize>> = Vec::new();
    for (i, &(a, b)) in segs.iter().enumerate() {
        let mut ts = cuts[i].clone();
        ts.sort_by(f64::total_cmp);
        let ids: Vec<usize> = ts.iter().map(|&t| vertex(lerp(a, b, t), &mut verts)).collect();
        adj.resize(verts.len(), Vec::new());
        for w in ids.windows(2) {
            let (u, v) = (w[0], w[1]);
            if u != v && !adj[u].contains(&v) {
                adj[u].push(v);
                adj[v].push(u);
            }
        }
    }
    (verts, adj)
}

/// The bounded faces of a planar graph, each counter-clockwise.
fn faces(verts: &[Point], adj: &[Vec<usize>]) -> Vec<Vec<Point>> {
    let angle = |u: usize, v: usize| {
        let d = sub(verts[v], verts[u]);
        d.1.atan2(d.0)
    };
    let sorted: Vec<Vec<usize>> = adj
        .iter()
        .enumerate()
        .map(|(u, ns)| {
            let mut ns = ns.clone();
            ns.sort_by(|&a, &b| angle(u, a).total_cmp(&angle(u, b)));
            ns
        })
        .collect();
    let mut seen: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for u in 0..verts.len() {
        for &v in &sorted[u] {
            if seen.contains(&(u, v)) {
                continue;
            }
            let mut face = Vec::new();
            let (mut a, mut b) = (u, v);
            while seen.insert((a, b)) {
                face.push(verts[a]);
                // The face on the left turns as sharply clockwise as it can at each vertex.
                let ns = &sorted[b];
                let k = ns.iter().position(|&x| x == a).expect("edge is in both lists");
                let c = ns[(k + ns.len() - 1) % ns.len()];
                (a, b) = (b, c);
            }
            if area(&face) > 1e-7 {
                out.push(face);
            }
        }
    }
    out
}

/// A polygon's signed area, positive counter-clockwise.
fn area(poly: &[Point]) -> f64 {
    (0..poly.len()).map(|i| cross(poly[i], poly[(i + 1) % poly.len()])).sum::<f64>() / 2.0
}

fn centroid(poly: &[Point]) -> Point {
    let a = area(poly);
    let (mut x, mut y) = (0.0, 0.0);
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let c = cross(p, q);
        x += (p.0 + q.0) * c;
        y += (p.1 + q.1) * c;
    }
    (x / (6.0 * a), y / (6.0 * a))
}

fn ring(r: f64) -> Vec<Point> {
    (0..RING_SEGMENTS).map(|i| polar(r, PI / 2.0 + 2.0 * PI * i as f64 / RING_SEGMENTS as f64)).collect()
}

/// A place where one band passes over another.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crossing {
    pub at: Point,
    /// The upper band's direction there.
    pub dir: Point,
    /// How far the upper band runs straight before and after the crossing.
    pub room: (f64, f64),
    /// The sine of the angle between the bands.
    pub sin: f64,
}

/// A tier's badge in the unit space.
#[derive(Debug, Clone, PartialEq)]
pub struct Badge {
    pub style: Style,
    /// The tiles or panels between the bands.
    pub fills: Vec<Fill>,
    /// The bands of the pattern, then the border rings.
    pub strokes: Vec<Stroke>,
    /// Where the bands weave, each over-and-under once.
    pub crossings: Vec<Crossing>,
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

/// The straight runs of a strand, closing it if it is a loop.
fn runs(points: &[Point], closed: bool) -> Vec<(Point, Point)> {
    let mut out: Vec<(Point, Point)> = points.windows(2).map(|w| (w[0], w[1])).collect();
    if closed && points.len() > 2 {
        out.push((points[points.len() - 1], points[0]));
    }
    out
}

/// Where the strands cross, with the upper one chosen so that every strand goes over and under in turn.
fn weave(strands: &[(Vec<Point>, bool)]) -> Vec<Crossing> {
    let segs: Vec<(usize, Point, Point)> = strands.iter().enumerate().flat_map(|(i, (p, c))| runs(p, *c).into_iter().map(move |(a, b)| (i, a, b))).collect();
    // Each crossing: point, the two segments, and how far along each strand it lies.
    struct X {
        at: Point,
        segs: [usize; 2],
    }
    let mut xs: Vec<X> = Vec::new();
    for i in 0..segs.len() {
        for j in i + 1..segs.len() {
            let ((_, a, b), (_, c, d)) = (segs[i], segs[j]);
            if let Some((t, u)) = params(a, sub(b, a), c, sub(d, c)) {
                let e = 1e-6;
                if t > e && t < 1.0 - e && u > e && u < 1.0 - e {
                    xs.push(X { at: lerp(a, b, t), segs: [i, j] });
                }
            }
        }
    }
    // Along each strand, its crossings in order.
    let mut along: Vec<Vec<(f64, usize, usize)>> = vec![Vec::new(); strands.len()];
    let start: Vec<usize> = {
        let mut s = vec![0; strands.len()];
        for (k, &(i, _, _)) in segs.iter().enumerate().rev() {
            s[i] = k;
        }
        s
    };
    for (x, c) in xs.iter().enumerate() {
        for side in 0..2 {
            let k = c.segs[side];
            let (strand, a, b) = segs[k];
            let pos = (k - start[strand]) as f64 + norm(sub(c.at, a)) / norm(sub(b, a));
            along[strand].push((pos, x, side));
        }
    }
    for l in &mut along {
        l.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    // over[x] = which side of crossing x is on top; neighbours along a strand alternate.
    let mut over: Vec<Option<usize>> = vec![None; xs.len()];
    let mut links: Vec<Vec<(usize, usize, usize)>> = vec![Vec::new(); xs.len()];
    for l in &along {
        for w in l.windows(2) {
            let ((_, x, sx), (_, y, sy)) = (w[0], w[1]);
            links[x].push((sx, y, sy));
            links[y].push((sy, x, sx));
        }
    }
    for seed in 0..xs.len() {
        if over[seed].is_some() {
            continue;
        }
        over[seed] = Some(0);
        let mut queue = vec![seed];
        while let Some(x) = queue.pop() {
            let top = over[x].expect("assigned");
            for &(sx, y, sy) in &links[x] {
                if over[y].is_none() {
                    // This strand is on top at x exactly when it is underneath at y.
                    let on_top = top == sx;
                    over[y] = Some(if on_top { 1 - sy } else { sy });
                    queue.push(y);
                }
            }
        }
    }
    xs.iter()
        .zip(over)
        .map(|(c, top)| {
            let top = top.expect("assigned");
            let (_, a, b) = segs[c.segs[top]];
            let (_, p, q) = segs[c.segs[1 - top]];
            let dir = unit(sub(b, a));
            Crossing { at: c.at, dir, room: (norm(sub(c.at, a)), norm(sub(b, c.at))), sin: cross(dir, unit(sub(q, p))).abs() }
        })
        .collect()
}

/// Builds a tier's badge.
pub fn badge(tier: usize) -> Badge {
    let rc = recipe(tier);
    let tiles = tiling(rc.fold, rc.rings, rc.scale);
    let segs = pattern(&tiles, rc.contact.to_radians(), REACH);
    let strands = clip(chain(&segs), RIM);
    let crossings = weave(&strands);

    // The panels between the bands: faces of the pattern and the rim's ring, inside the rim.
    let rim = ring(RIM);
    let mut all = segs.clone();
    all.extend((0..rim.len()).map(|i| (rim[i], rim[(i + 1) % rim.len()])));
    let (verts, adj) = arrange(&all);
    let mut panels: Vec<Vec<Point>> = faces(&verts, &adj).into_iter().filter(|f| f.iter().all(|&p| norm(p) <= RIM + 1e-6)).collect();
    // Pieces alike under the rosette's symmetry share a colour, given out from the centre.
    let key = |f: &Vec<Point>| ((norm(centroid(f)) * 1e3).round() as i64, (area(f) * 1e4).round() as i64);
    panels.sort_by(|a, b| key(a).cmp(&key(b)).then(centroid(a).1.total_cmp(&centroid(b).1)).then(centroid(a).0.total_cmp(&centroid(b).0)));
    let mut kinds: Vec<(i64, i64)> = panels.iter().map(key).collect();
    kinds.dedup();
    let fills = panels
        .into_iter()
        .map(|f| {
            let kind = kinds.iter().position(|k| *k == key(&f)).expect("a kind");
            let color = if rc.tiles.is_empty() { rc.palette.rosette } else { rc.tiles[kind % rc.tiles.len()] };
            Fill { points: f, color }
        })
        .collect();

    let mut strokes: Vec<Stroke> = strands.into_iter().map(|(points, closed)| Stroke { points, closed, role: Role::Strap }).collect();
    for r in [RIM, 0.97] {
        strokes.push(Stroke { points: ring(r), closed: true, role: Role::Ring });
    }
    Badge { style: rc.style, fills, strokes, crossings, palette: rc.palette }
}

fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

/// `a` moved towards `b` by `t`.
fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let m = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// A coordinate to two places, with no negative zero so equal shapes print equally.
fn num(v: f64) -> String {
    let s = format!("{v:.2}");
    if s == "-0.00" { "0.00".into() } else { s }
}

/// Maps the unit space onto the page.
struct Page {
    cx: f64,
    cy: f64,
    r: f64,
}

impl Page {
    /// An SVG path's `d` for a polyline, moved by `shift` (in unit space).
    fn path(&self, points: &[Point], closed: bool, shift: Point) -> String {
        let mut d = String::new();
        for (i, p) in points.iter().enumerate() {
            let _ = write!(d, "{}{} {}", if i == 0 { "M" } else { " L" }, num(self.cx + (p.0 + shift.0) * self.r), num(self.cy - (p.1 + shift.1) * self.r));
        }
        if closed {
            d.push_str(" Z");
        }
        d
    }

    fn circle(&self, r: f64, attrs: &str) -> String {
        format!("<circle cx=\"{}\" cy=\"{}\" r=\"{}\" {attrs}/>", num(self.cx), num(self.cy), num(r * self.r))
    }
}

/// One layer of a band: its colour, width and offset, in unit space.
struct Layer {
    color: Rgb,
    width: f64,
    shift: Point,
}

fn layer(color: Rgb, width: f64) -> Layer {
    Layer { color, width, shift: (0.0, 0.0) }
}

/// The light comes from the upper left, so a raised band's shadow falls down and to the right.
const SHADOW: Point = (0.014, -0.018);

/// A style's band layers, bottom first.
fn layers(b: &Badge) -> Vec<Layer> {
    let p = b.palette;
    match b.style {
        Style::Carved => vec![
            Layer { color: mix(p.rosette, p.band, 0.7), width: 0.066, shift: SHADOW },
            layer(mix(p.line, p.band, 0.55), 0.064),
            layer(p.line, 0.052),
            layer(mix(p.line, p.band, 0.45), 0.008),
        ],
        Style::Zellij => vec![layer(p.band, 0.058), layer(p.line, 0.034)],
        Style::Girih => vec![layer(p.band, 0.056), layer(p.line, 0.036), layer(mix(p.line, p.band, 0.55), 0.007)],
    }
}

/// A tier's badge as an SVG `<g>`, centred on (cx, cy) with radius r: the ground disc, the panels or tiles, the woven bands and the border.
pub fn svg(tier: usize, cx: f64, cy: f64, r: f64) -> String {
    let tier = tier.min(TIERS - 1);
    let b = badge(tier);
    let p = b.palette;
    let page = Page { cx, cy, r };
    let mut out = String::from("<g>");
    let id = |name: &str| format!("badge{tier}-{name}");
    let _ = write!(
        out,
        "<defs><radialGradient id=\"{}\" cx=\"0.36\" cy=\"0.3\" r=\"0.8\"><stop offset=\"0\" stop-color=\"#fff\" stop-opacity=\"{}\"/><stop offset=\"0.55\" stop-color=\"#fff\" stop-opacity=\"0\"/><stop offset=\"1\" stop-color=\"#000\" stop-opacity=\"{}\"/></radialGradient></defs>",
        id("light"),
        if b.style == Style::Carved { 0.28 } else { 0.18 },
        if b.style == Style::Carved { 0.34 } else { 0.24 }
    );
    out.push_str(&page.circle(1.0, &format!("fill=\"{}\"", hex(p.ground))));

    // The panels: a carved one is a recess, in shadow along its upper-left walls; a tile is flat glaze.
    for f in &b.fills {
        if b.style == Style::Carved {
            let _ = write!(out, "<path d=\"{}\" fill=\"{}\"/>", page.path(&f.points, true, (0.0, 0.0)), hex(mix(f.color, p.band, 0.55)));
            let _ = write!(out, "<path d=\"{}\" fill=\"{}\"/>", page.path(&f.points, true, SHADOW), hex(f.color));
        } else {
            let _ = write!(out, "<path d=\"{}\" fill=\"{}\"/>", page.path(&f.points, true, (0.0, 0.0)), hex(f.color));
        }
    }

    // A carved roundel's centre panel holds a raised boss: a small star of the rosette's fold.
    if b.style == Style::Carved
        && let Some(centre) = b.fills.first()
    {
        let n = recipe(tier).fold;
        let reach = centre.points.iter().map(|&q| norm(q)).fold(f64::INFINITY, f64::min);
        let star = hankin_star(n, reach * 0.8, PI / 2.0, star_contact(n, n / 4 + 1)).expect("contact in bounds");
        let _ = write!(out, "<path d=\"{}\" fill=\"{}\"/>", page.path(&star, true, SHADOW), hex(mix(p.rosette, p.band, 0.7)));
        let _ = write!(out, "<path d=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\" stroke-linejoin=\"miter\"/>", page.path(&star, true, (0.0, 0.0)), hex(p.line), hex(mix(p.line, p.band, 0.55)), num(0.008 * r));
        out.push_str(&page.circle(reach * 0.2, &format!("fill=\"{}\"", hex(mix(p.line, p.band, 0.3)))));
    }

    border(&mut out, &b, &page);

    // The bands: every layer for all of them, then each crossing's upper band again on top, then the rings.
    let layers = layers(&b);
    let group = |out: &mut String, l: &Layer, paths: Vec<String>| {
        let _ = write!(out, "<g fill=\"none\" stroke=\"{}\" stroke-width=\"{}\" stroke-linejoin=\"miter\" stroke-miterlimit=\"8\">", hex(l.color), num(l.width * r));
        for d in paths {
            let _ = write!(out, "<path d=\"{d}\"/>");
        }
        out.push_str("</g>");
    };
    let strokes = |role: Role, l: &Layer| b.strokes.iter().filter(|s| s.role == role).map(|s| page.path(&s.points, s.closed, l.shift)).collect::<Vec<_>>();
    for l in &layers {
        group(&mut out, l, strokes(Role::Strap, l));
    }
    // A crossing redraws the unshifted layers only: a cast shadow would show past the piece's ends.
    let upper: Vec<&Layer> = layers.iter().filter(|l| l.shift == (0.0, 0.0)).collect();
    let outer = upper.iter().map(|l| l.width).fold(0.0, f64::max);
    for l in &upper {
        // Long enough to cover the lower band, short of the upper band's next bend; the layers narrower than the outline run a little further, over its cut ends.
        let extra = outer * if l.width < outer * 0.9 { 0.6 } else { 0.25 };
        let pieces = b.crossings.iter().map(|c| {
            let half = outer / 2.0 / c.sin.max(0.2) + extra;
            let (back, ahead) = (half.min(c.room.0 - outer * 0.5).max(0.0), half.min(c.room.1 - outer * 0.5).max(0.0));
            page.path(&[sub(c.at, mul(c.dir, back)), add(c.at, mul(c.dir, ahead))], false, (0.0, 0.0))
        });
        group(&mut out, l, pieces.collect());
    }
    for l in &layers {
        group(&mut out, l, strokes(Role::Ring, l));
    }
    out.push_str(&page.circle(1.0, &format!("fill=\"url(#{})\"", id("light"))));
    out.push_str(&page.circle(1.0, &format!("fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"", hex(p.band), num(0.012 * r))));
    out.push_str("</g>");
    out
}

/// The border between the pattern's rim and the edge: a carved chain of lozenges, a ring of small cut tiles, or a chain of gold stars.
fn border(out: &mut String, b: &Badge, page: &Page) {
    let p = b.palette;
    let (inner, outer) = (RIM, 0.97);
    let mid = (inner + outer) / 2.0;
    let depth = outer - inner;
    match b.style {
        Style::Carved => {
            out.push_str(&page.circle(mid, &format!("fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"", hex(mix(p.rosette, p.band, 0.25)), num(depth * page.r))));
            let n = 48;
            let mut d = String::new();
            for i in 0..n {
                let a = PI / 2.0 + 2.0 * PI * i as f64 / n as f64;
                let half = PI / n as f64 * 0.8;
                let lozenge = [polar(mid, a - half), polar(mid + depth * 0.28, a), polar(mid, a + half), polar(mid - depth * 0.28, a)];
                d.push_str(&page.path(&lozenge, true, (0.0, 0.0)));
            }
            let _ = write!(out, "<path d=\"{}\" fill=\"{}\" transform=\"translate({} {})\"/>", d, hex(p.band), num(SHADOW.0 * page.r * 0.5), num(-SHADOW.1 * page.r * 0.5));
            let _ = write!(out, "<path d=\"{d}\" fill=\"{}\"/>", hex(p.line));
        }
        Style::Zellij => {
            let n = 64;
            let colors = [b.fills.first().map_or(p.rosette, |f| f.color), p.field, p.line, p.rosette];
            for i in 0..n {
                let (a0, a1) = (PI / 2.0 + 2.0 * PI * i as f64 / n as f64, PI / 2.0 + 2.0 * PI * (i + 1) as f64 / n as f64);
                // Triangles point in and out in turn, a sawtooth of cut tiles.
                let up = [polar(inner, a0), polar(inner, a1), polar(outer, (a0 + a1) / 2.0)];
                let down = [polar(outer, (a0 + a1) / 2.0), polar(outer, a1 + (a1 - a0) / 2.0), polar(inner, a1)];
                let c1 = if i % 2 == 0 { p.field } else { p.rosette };
                let _ = write!(out, "<path d=\"{}\" fill=\"{}\"/>", page.path(&up, true, (0.0, 0.0)), hex(c1));
                let _ = write!(out, "<path d=\"{}\" fill=\"{}\"/>", page.path(&down, true, (0.0, 0.0)), hex(colors[2]));
            }
            let mut grout = String::new();
            for i in 0..n {
                let (a0, a1) = (PI / 2.0 + 2.0 * PI * i as f64 / n as f64, PI / 2.0 + 2.0 * PI * (i + 1) as f64 / n as f64);
                grout.push_str(&page.path(&[polar(inner, a0), polar(outer, (a0 + a1) / 2.0), polar(inner, a1)], false, (0.0, 0.0)));
            }
            let _ = write!(out, "<path d=\"{grout}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\" stroke-linejoin=\"round\"/>", hex(p.band), num(0.006 * page.r));
        }
        Style::Girih => {
            out.push_str(&page.circle(mid, &format!("fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"", hex(p.ground), num(depth * page.r))));
            let n = 40;
            let mut stars = String::new();
            let mut beads = String::new();
            for i in 0..n {
                let a = PI / 2.0 + 2.0 * PI * i as f64 / n as f64;
                let c = polar(mid, a);
                let star = hankin_star(10, depth * 0.36, a, star_contact(10, 3)).expect("contact in bounds");
                stars.push_str(&page.path(&star.iter().map(|&q| add(c, q)).collect::<Vec<_>>(), true, (0.0, 0.0)));
                let at = polar(mid, a + PI / n as f64);
                let bead = [add(at, polar(depth * 0.12, a + PI / n as f64)), add(at, polar(depth * 0.12, a + PI / n as f64 + PI / 2.0)), add(at, polar(depth * 0.12, a + PI / n as f64 + PI)), add(at, polar(depth * 0.12, a + PI / n as f64 - PI / 2.0))];
                beads.push_str(&page.path(&bead, true, (0.0, 0.0)));
            }
            let _ = write!(out, "<path d=\"{stars}\" fill=\"{}\"/><path d=\"{beads}\" fill=\"{}\"/>", hex(p.line), hex(p.rosette));
        }
    }
}

/// The {n/k} star: contact πk/n sends each ray to the tip k steps on.
fn star_contact(n: usize, k: usize) -> f64 {
    PI * k as f64 / n as f64
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
    fn tiers_follow_carved_then_zellij_then_girih() {
        let styles: Vec<Style> = (0..TIERS).map(|t| recipe(t).style).collect();
        use Style::*;
        assert_eq!(styles, [Carved, Carved, Zellij, Zellij, Zellij, Girih, Girih]);
        let folds: Vec<usize> = (0..TIERS).map(|t| recipe(t).fold).collect();
        assert_eq!(folds, [8, 12, 8, 8, 8, 10, 10], "girih is ten-fold");
    }

    #[test]
    fn rosette_tiling_closes_without_gaps() {
        for (n, rings) in [(8, 1), (8, 2), (10, 1), (12, 1)] {
            let tiles = tiling(n, rings, 0.3);
            // Every tile is convex and counter-clockwise, and at every inner vertex the angles sum to a full turn.
            for t in &tiles {
                assert!(area(t) > 0.0, "n {n}: a tile runs clockwise");
                for i in 0..t.len() {
                    let (a, b, c) = (t[i], t[(i + 1) % t.len()], t[(i + 2) % t.len()]);
                    assert!(cross(sub(b, a), sub(c, b)) > 0.0, "n {n}: a tile is not convex");
                }
            }
            let centre = &tiles[0];
            for &v in centre {
                let turn: f64 = tiles
                    .iter()
                    .filter_map(|t| {
                        let i = t.iter().position(|&p| close(p, v))?;
                        let (a, c) = (t[(i + t.len() - 1) % t.len()], t[(i + 1) % t.len()]);
                        Some(dot(unit(sub(a, v)), unit(sub(c, v))).clamp(-1.0, 1.0).acos())
                    })
                    .sum();
                assert!((turn - 2.0 * PI).abs() < 1e-9, "n {n}: {turn} around a centre vertex");
            }
        }
        // With two rings of n = 8 the notch octagons meet the pentagons' apexes edge to edge.
        let tiles = tiling(8, 2, 0.3);
        let apex = tiles[1][3];
        assert_eq!(tiles.iter().filter(|t| t.iter().any(|&p| close(p, apex))).count(), 3);
    }

    #[test]
    fn badge_has_the_rosettes_rotational_symmetry() {
        for tier in 0..TIERS {
            let b = badge(tier);
            let n = recipe(tier).fold;
            let segs = b.segments();
            let turn = 2.0 * PI / n as f64;
            // Every segment, turned one step, lands on a segment of the badge.
            let keyed = |p: Point| ((p.0 * 1e5).round() as i64, (p.1 * 1e5).round() as i64);
            let mut points: std::collections::HashSet<(i64, i64)> = std::collections::HashSet::new();
            for &(a, b) in &segs {
                points.insert(keyed(a));
                points.insert(keyed(b));
            }
            let strap: Vec<Point> = b.strokes.iter().filter(|s| s.role == Role::Strap).flat_map(|s| s.points.iter().copied()).collect();
            let misses = strap.iter().filter(|&&p| {
                let q = keyed(rotate(p, turn));
                !(-1..=1).any(|dx| (-1..=1).any(|dy| points.contains(&(q.0 + dx, q.1 + dy))))
            });
            assert_eq!(misses.count(), 0, "tier {tier}");
        }
    }

    #[test]
    fn panels_tile_the_disc_inside_the_rim() {
        for tier in 0..TIERS {
            let b = badge(tier);
            let total: f64 = b.fills.iter().map(|f| area(&f.points)).sum();
            let disc = area(&ring(RIM));
            assert!((total - disc).abs() < 1e-6, "tier {tier}: panels cover {total} of {disc}");
            assert!(b.fills.len() > 3 * recipe(tier).fold, "tier {tier}: only {} panels", b.fills.len());
        }
    }

    #[test]
    fn bands_weave_over_and_under_in_turn() {
        for tier in 0..TIERS {
            let b = badge(tier);
            assert!(!b.crossings.is_empty(), "tier {tier} has no crossings");
            let straps: Vec<&Stroke> = b.strokes.iter().filter(|s| s.role == Role::Strap).collect();
            // Walking each band, it is on top at every other crossing.
            for s in &straps {
                let mut seen: Vec<(f64, bool)> = Vec::new();
                for (k, (a, c)) in runs(&s.points, s.closed).into_iter().enumerate() {
                    for x in &b.crossings {
                        let d = sub(c, a);
                        let t = dot(sub(x.at, a), d) / dot(d, d);
                        let off = cross(unit(d), sub(x.at, a)).abs();
                        if off < 1e-9 && t > 1e-6 && t < 1.0 - 1e-6 {
                            seen.push((k as f64 + t, cross(x.dir, unit(d)).abs() < 1e-6));
                        }
                    }
                }
                seen.sort_by(|a, b| a.0.total_cmp(&b.0));
                for w in seen.windows(2) {
                    assert_ne!(w[0].1, w[1].1, "tier {tier}: a band goes over or under twice in a row");
                }
            }
        }
    }

    #[test]
    fn same_tier_yields_identical_svg() {
        for tier in 0..TIERS {
            assert_eq!(svg(tier, 120.0, 120.0, 100.0), svg(tier, 120.0, 120.0, 100.0));
            assert!(!svg(tier, 120.0, 120.0, 100.0).is_empty());
        }
    }

    #[test]
    fn higher_tiers_have_strictly_more_segments_within_a_tradition() {
        // Each tradition starts plainer than the last one ended, so the count grows within a tradition, and Grandmaster has the most.
        let counts: Vec<usize> = (0..TIERS).map(|t| badge(t).segments().len()).collect();
        assert!(counts[0] > 0);
        for t in 1..TIERS {
            if recipe(t).style == recipe(t - 1).style {
                assert!(counts[t - 1] < counts[t], "segment counts {counts:?}");
            }
        }
        assert_eq!(counts.iter().max(), counts.last(), "segment counts {counts:?}");
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
            for s in badge(tier).strokes.iter().filter(|s| s.role == Role::Strap) {
                assert!(s.points.iter().all(|&p| dist(p) <= RIM + 1e-9), "tier {tier}: a band runs past the rim");
            }
        }
    }

    #[test]
    #[ignore = "writes badges.svg, badges.png and badge-tierN.png for every tier into the folder $BADGE_PREVIEW"]
    fn preview_all_tiers() {
        use resvg::{tiny_skia, usvg};
        let dir = std::path::PathBuf::from(std::env::var("BADGE_PREVIEW").expect("set BADGE_PREVIEW"));
        std::fs::create_dir_all(&dir).unwrap();
        let render = |svg: &str, w: u32, h: u32, path: &std::path::Path| {
            let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).unwrap();
            let mut pixmap = tiny_skia::Pixmap::new(w, h).unwrap();
            resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());
            pixmap.save_png(path).unwrap();
        };
        let mut row = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1680\" height=\"260\" viewBox=\"0 0 1680 260\"><rect width=\"1680\" height=\"260\" fill=\"#f8f3e6\"/>");
        for tier in 0..TIERS {
            row.push_str(&svg(tier, 120.0 + 240.0 * tier as f64, 130.0, 110.0));
            let one = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"600\" height=\"600\" viewBox=\"0 0 600 600\"><rect width=\"600\" height=\"600\" fill=\"#f8f3e6\"/>{}</svg>", svg(tier, 300.0, 300.0, 280.0));
            render(&one, 600, 600, &dir.join(format!("badge-tier{tier}.png")));
        }
        row.push_str("</svg>");
        std::fs::write(dir.join("badges.svg"), &row).unwrap();
        render(&row, 1680, 260, &dir.join("badges.png"));
    }
}
