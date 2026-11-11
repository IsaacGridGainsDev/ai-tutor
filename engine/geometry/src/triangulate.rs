//! Dividing a planar face with holes into triangles, by joining each hole to the outer loop with a
//! bridge and then cutting ears off the resulting single loop.

use crate::{Face, GeometryError, Point, TOLERANCE};

type Flat = (f64, f64);

fn cross(origin: Flat, a: Flat, b: Flat) -> f64 {
    (a.0 - origin.0) * (b.1 - origin.1) - (a.1 - origin.1) * (b.0 - origin.0)
}

/// Whether two segments cross somewhere other than at a shared end.
fn segments_cross(a: Flat, b: Flat, c: Flat, d: Flat) -> bool {
    let same = |p: Flat, q: Flat| (p.0 - q.0).abs() < TOLERANCE && (p.1 - q.1).abs() < TOLERANCE;
    if same(a, c) || same(a, d) || same(b, c) || same(b, d) {
        return false;
    }
    let (d1, d2, d3, d4) = (cross(c, d, a), cross(c, d, b), cross(a, b, c), cross(a, b, d));
    (d1 > TOLERANCE && d2 < -TOLERANCE || d1 < -TOLERANCE && d2 > TOLERANCE)
        && (d3 > TOLERANCE && d4 < -TOLERANCE || d3 < -TOLERANCE && d4 > TOLERANCE)
}

fn inside_triangle(point: Flat, a: Flat, b: Flat, c: Flat) -> bool {
    cross(a, b, point) > -TOLERANCE && cross(b, c, point) > -TOLERANCE && cross(c, a, point) > -TOLERANCE
}

/// Triangles of a face as vertex indices, wound like the face's outer loop.
pub(crate) fn face(vertices: &[Point], face: &Face, normal: Point) -> Result<Vec<[usize; 3]>, GeometryError> {
    // Two axes in the face's plane, chosen so that an anticlockwise loop stays anticlockwise.
    let helper = if normal.x.abs() < 0.9 { Point::new(1.0, 0.0, 0.0) } else { Point::new(0.0, 1.0, 0.0) };
    let u = normal.cross(helper).normalized().ok_or(GeometryError::DegenerateLoop)?;
    let v = normal.cross(u);
    let flat = |index: usize| -> Flat { (vertices[index].dot(u), vertices[index].dot(v)) };

    // Join the holes to the outer loop, the one reaching furthest right first.
    let mut ring: Vec<usize> = face.outer.clone();
    let mut holes: Vec<&Vec<usize>> = face.holes.iter().collect();
    holes.sort_by(|a, b| {
        let reach = |hole: &Vec<usize>| hole.iter().map(|&index| flat(index).0).fold(f64::MIN, f64::max);
        reach(b).total_cmp(&reach(a))
    });
    let every_edge: Vec<(usize, usize)> = std::iter::once(&face.outer)
        .chain(face.holes.iter())
        .flat_map(|ring| (0..ring.len()).map(move |index| (ring[index], ring[(index + 1) % ring.len()])))
        .collect();
    for hole in holes {
        let start = (0..hole.len()).max_by(|&a, &b| flat(hole[a]).0.total_cmp(&flat(hole[b]).0)).ok_or(GeometryError::DegenerateLoop)?;
        let from = flat(hole[start]);
        // The nearest corner of the loop that can be reached without crossing any edge.
        let mut candidates: Vec<usize> = (0..ring.len()).collect();
        candidates.sort_by(|&a, &b| {
            let distance = |position: usize| {
                let point = flat(ring[position]);
                (point.0 - from.0).powi(2) + (point.1 - from.1).powi(2)
            };
            distance(a).total_cmp(&distance(b))
        });
        let bridge = candidates
            .into_iter()
            .find(|&position| {
                let to = flat(ring[position]);
                let blocked = every_edge.iter().any(|&(a, b)| segments_cross(from, to, flat(a), flat(b)));
                // A bridge laid earlier is also an edge of the loop now.
                let over_bridge = (0..ring.len()).any(|i| segments_cross(from, to, flat(ring[i]), flat(ring[(i + 1) % ring.len()])));
                !blocked && !over_bridge
            })
            .ok_or(GeometryError::CannotTriangulate)?;
        // Walk to the bridge, round the hole and back: the loop now visits both bridge ends twice.
        let mut joined = Vec::with_capacity(ring.len() + hole.len() + 2);
        joined.extend_from_slice(&ring[..=bridge]);
        joined.extend((0..=hole.len()).map(|step| hole[(start + step) % hole.len()]));
        joined.extend_from_slice(&ring[bridge..]);
        ring = joined;
    }

    // Cut ears: a convex corner whose triangle holds no other corner of the loop.
    let mut triangles = Vec::with_capacity(ring.len().saturating_sub(2));
    let mut guard = ring.len() * ring.len() + 8;
    while ring.len() > 3 {
        let count = ring.len();
        let ear = (0..count).find(|&position| {
            let (a, b, c) = (ring[(position + count - 1) % count], ring[position], ring[(position + 1) % count]);
            let (fa, fb, fc) = (flat(a), flat(b), flat(c));
            if cross(fa, fb, fc) <= TOLERANCE {
                return false;
            }
            !ring.iter().any(|&other| {
                let point = flat(other);
                let is_corner = [fa, fb, fc].iter().any(|corner| (corner.0 - point.0).abs() < TOLERANCE && (corner.1 - point.1).abs() < TOLERANCE);
                !is_corner && inside_triangle(point, fa, fb, fc)
            })
        });
        // Corners in a straight line enclose nothing and can simply be dropped.
        let straight = || (0..count).find(|&position| {
            cross(flat(ring[(position + count - 1) % count]), flat(ring[position]), flat(ring[(position + 1) % count])).abs() <= TOLERANCE
        });
        match ear {
            Some(position) => {
                triangles.push([ring[(position + count - 1) % count], ring[position], ring[(position + 1) % count]]);
                ring.remove(position);
            }
            None => {
                let position = straight().ok_or(GeometryError::CannotTriangulate)?;
                ring.remove(position);
            }
        }
        guard -= 1;
        if guard == 0 {
            return Err(GeometryError::CannotTriangulate);
        }
    }
    if ring.len() == 3 && cross(flat(ring[0]), flat(ring[1]), flat(ring[2])).abs() > TOLERANCE {
        triangles.push([ring[0], ring[1], ring[2]]);
    }
    if triangles.is_empty() {
        return Err(GeometryError::DegenerateLoop);
    }
    Ok(triangles)
}
