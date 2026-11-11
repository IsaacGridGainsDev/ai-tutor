//! Flat-faced solid modelling.
//!
//! The model follows the one SketchUp exposes through its public API (see
//! `docs/research/3d-engines.md`): a solid is a set of planar faces, each an outer loop with any
//! number of holes, sharing vertices. Triangles and edge lines are derived for drawing and are
//! never the model itself.

pub mod room;
mod triangulate;

use std::collections::HashMap;
use std::fmt;
use std::ops::{Add, Mul, Sub};

use serde::Serialize;

/// Lengths closer than this are treated as equal.
pub const TOLERANCE: f64 = 1e-7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    pub fn normalized(self) -> Option<Self> {
        let length = self.length();
        (length > TOLERANCE).then(|| self * (1.0 / length))
    }
}

impl Add for Point {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }
}

impl Sub for Point {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }
}

impl Mul<f64> for Point {
    type Output = Self;
    fn mul(self, scale: f64) -> Self {
        Self::new(self.x * scale, self.y * scale, self.z * scale)
    }
}

#[derive(Debug, PartialEq)]
pub enum GeometryError {
    /// A loop needs at least three distinct corners enclosing some area.
    DegenerateLoop,
    /// The corners of a face do not lie in one plane.
    NotPlanar,
    /// A face cannot be pushed by no distance.
    ZeroDistance,
    /// A face could not be divided into triangles, usually because its boundary crosses itself.
    CannotTriangulate,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DegenerateLoop => "A face needs at least three corners that enclose an area.",
            Self::NotPlanar => "The corners of a face do not lie in one plane.",
            Self::ZeroDistance => "A face cannot be pushed by no distance.",
            Self::CannotTriangulate => "A face could not be divided into triangles; its boundary may cross itself.",
        })
    }
}

impl std::error::Error for GeometryError {}

/// A planar face: an outer loop, and holes wound the opposite way. Entries index `Solid::vertices`.
/// Seen from outside the solid, the outer loop runs anticlockwise.
#[derive(Clone, Debug)]
pub struct Face {
    pub outer: Vec<usize>,
    pub holes: Vec<Vec<usize>>,
}

impl Face {
    fn loops(&self) -> impl Iterator<Item = &Vec<usize>> {
        std::iter::once(&self.outer).chain(self.holes.iter())
    }
}

/// Triangles and edge lines derived from a solid, ready for a GPU.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Mesh {
    /// Three corners per triangle, three numbers per corner.
    pub positions: Vec<f32>,
    /// One normal per corner; every corner of a face shares the face's normal, so faces look flat.
    pub normals: Vec<f32>,
    /// Two ends per edge, three numbers per end: every edge of every face, once.
    pub edges: Vec<f32>,
}

#[derive(Clone, Debug, Default)]
pub struct Solid {
    pub vertices: Vec<Point>,
    pub faces: Vec<Face>,
}

/// The normal of a loop by Newell's method, scaled by twice the enclosed area.
pub(crate) fn loop_normal(points: &[Point]) -> Point {
    let mut normal = Point::new(0.0, 0.0, 0.0);
    for (index, &current) in points.iter().enumerate() {
        let next = points[(index + 1) % points.len()];
        normal.x += (current.y - next.y) * (current.z + next.z);
        normal.y += (current.z - next.z) * (current.x + next.x);
        normal.z += (current.x - next.x) * (current.y + next.y);
    }
    normal
}

fn check_planar(points: &[Point], origin: Point, normal: Point) -> Result<(), GeometryError> {
    let scale = points.iter().map(|point| (*point - origin).length()).fold(1.0, f64::max);
    if points.iter().any(|point| (*point - origin).dot(normal).abs() > TOLERANCE * 1e3 * scale) {
        return Err(GeometryError::NotPlanar);
    }
    Ok(())
}

impl Solid {
    /// Push a flat profile along its normal to make a solid, carrying any holes through it.
    ///
    /// The profile's normal follows the right-hand rule from the order of `outer`; a negative
    /// distance pushes the other way. Holes may be wound either way.
    pub fn extrude(outer: &[Point], holes: &[Vec<Point>], distance: f64) -> Result<Self, GeometryError> {
        if distance.abs() < TOLERANCE {
            return Err(GeometryError::ZeroDistance);
        }
        let mut outer = outer.to_vec();
        if distance < 0.0 {
            outer.reverse();
        }
        if outer.len() < 3 {
            return Err(GeometryError::DegenerateLoop);
        }
        let normal = loop_normal(&outer).normalized().ok_or(GeometryError::DegenerateLoop)?;
        check_planar(&outer, outer[0], normal)?;

        // Holes run the opposite way round to the outer loop.
        let mut wound = Vec::with_capacity(holes.len());
        for hole in holes {
            let mut hole = hole.clone();
            if hole.len() < 3 {
                return Err(GeometryError::DegenerateLoop);
            }
            check_planar(&hole, outer[0], normal)?;
            let facing = loop_normal(&hole).dot(normal);
            if facing.abs() < TOLERANCE {
                return Err(GeometryError::DegenerateLoop);
            }
            if facing > 0.0 {
                hole.reverse();
            }
            wound.push(hole);
        }

        let offset = normal * distance.abs();
        let mut solid = Self::default();
        let add_loop = |points: &[Point], solid: &mut Self| -> (Vec<usize>, Vec<usize>) {
            let start = solid.vertices.len();
            solid.vertices.extend(points.iter().copied());
            solid.vertices.extend(points.iter().map(|point| *point + offset));
            let count = points.len();
            ((start..start + count).collect(), (start + count..start + 2 * count).collect())
        };
        let (outer_near, outer_far) = add_loop(&outer, &mut solid);
        let hole_loops: Vec<(Vec<usize>, Vec<usize>)> = wound.iter().map(|hole| add_loop(hole, &mut solid)).collect();

        // The far cap keeps the profile's winding; the near cap faces the other way.
        let reversed = |ring: &[usize]| ring.iter().rev().copied().collect::<Vec<_>>();
        solid.faces.push(Face {
            outer: outer_far.clone(),
            holes: hole_loops.iter().map(|(_, far)| far.clone()).collect(),
        });
        solid.faces.push(Face {
            outer: reversed(&outer_near),
            holes: hole_loops.iter().map(|(near, _)| reversed(near)).collect(),
        });
        // One side face per profile edge. The same rule turns hole sides to face into the hole.
        for (near, far) in std::iter::once(&(outer_near, outer_far)).chain(hole_loops.iter()) {
            for index in 0..near.len() {
                let next = (index + 1) % near.len();
                solid.faces.push(Face { outer: vec![near[index], near[next], far[next], far[index]], holes: Vec::new() });
            }
        }
        Ok(solid)
    }

    fn points(&self, ring: &[usize]) -> Vec<Point> {
        ring.iter().map(|&index| self.vertices[index]).collect()
    }

    /// The outward unit normal of a face.
    pub fn face_normal(&self, face: &Face) -> Point {
        loop_normal(&self.points(&face.outer)).normalized().unwrap_or(Point::new(0.0, 0.0, 1.0))
    }

    /// Every edge once, as a pair of vertex indices with the smaller first.
    pub fn edges(&self) -> Vec<(usize, usize)> {
        let mut edges: Vec<(usize, usize)> = self
            .faces
            .iter()
            .flat_map(Face::loops)
            .flat_map(|ring| (0..ring.len()).map(move |index| (ring[index], ring[(index + 1) % ring.len()])))
            .map(|(from, to)| (from.min(to), from.max(to)))
            .collect();
        edges.sort_unstable();
        edges.dedup();
        edges
    }

    /// Whether the faces enclose a volume with no gaps: every edge is used by exactly two faces,
    /// once in each direction.
    pub fn is_manifold(&self) -> bool {
        let mut uses: HashMap<(usize, usize), u32> = HashMap::new();
        for ring in self.faces.iter().flat_map(Face::loops) {
            for index in 0..ring.len() {
                *uses.entry((ring[index], ring[(index + 1) % ring.len()])).or_default() += 1;
            }
        }
        !uses.is_empty() && uses.iter().all(|(&(from, to), &count)| count == 1 && uses.get(&(to, from)) == Some(&1))
    }

    /// Each face divided into triangles of vertex indices, wound like the face.
    pub fn triangles(&self) -> Result<Vec<(usize, [usize; 3])>, GeometryError> {
        let mut triangles = Vec::new();
        for (index, face) in self.faces.iter().enumerate() {
            let normal = self.face_normal(face);
            for triangle in triangulate::face(&self.vertices, face, normal)? {
                triangles.push((index, triangle));
            }
        }
        Ok(triangles)
    }

    /// Total area of all faces.
    pub fn area(&self) -> Result<f64, GeometryError> {
        Ok(self.triangles()?.iter().map(|(_, [a, b, c])| {
            (self.vertices[*b] - self.vertices[*a]).cross(self.vertices[*c] - self.vertices[*a]).length() / 2.0
        }).sum())
    }

    /// The enclosed volume. Meaningful only when the solid is manifold.
    pub fn volume(&self) -> Result<f64, GeometryError> {
        Ok(self.triangles()?.iter().map(|(_, [a, b, c])| {
            self.vertices[*a].dot(self.vertices[*b].cross(self.vertices[*c])) / 6.0
        }).sum())
    }

    pub fn mesh(&self) -> Result<Mesh, GeometryError> {
        let mut mesh = Mesh::default();
        for (face, corners) in self.triangles()? {
            let normal = self.face_normal(&self.faces[face]);
            for corner in corners {
                let point = self.vertices[corner];
                mesh.positions.extend([point.x as f32, point.y as f32, point.z as f32]);
                mesh.normals.extend([normal.x as f32, normal.y as f32, normal.z as f32]);
            }
        }
        for (from, to) in self.edges() {
            for point in [self.vertices[from], self.vertices[to]] {
                mesh.edges.extend([point.x as f32, point.y as f32, point.z as f32]);
            }
        }
        Ok(mesh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(width: f64, height: f64) -> Vec<Point> {
        vec![Point::new(0.0, 0.0, 0.0), Point::new(width, 0.0, 0.0), Point::new(width, height, 0.0), Point::new(0.0, height, 0.0)]
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn pushing_a_rectangle_makes_a_closed_box() {
        let solid = Solid::extrude(&rectangle(2.0, 3.0), &[], 4.0).unwrap();
        assert_eq!((solid.vertices.len(), solid.faces.len(), solid.edges().len()), (8, 6, 12));
        assert!(solid.is_manifold());
        assert!(close(solid.volume().unwrap(), 24.0));
        assert!(close(solid.area().unwrap(), 2.0 * (6.0 + 8.0 + 12.0)));
    }

    #[test]
    fn a_hole_is_carried_through_the_solid() {
        let hole = vec![Point::new(0.5, 0.5, 0.0), Point::new(1.5, 0.5, 0.0), Point::new(1.5, 2.0, 0.0), Point::new(0.5, 2.0, 0.0)];
        let solid = Solid::extrude(&rectangle(2.0, 3.0), &[hole.clone()], 0.2).unwrap();
        assert_eq!(solid.faces.len(), 2 + 4 + 4);
        assert!(solid.is_manifold());
        assert!(close(solid.volume().unwrap(), (6.0 - 1.5) * 0.2));
        // The hole may be given wound either way.
        let reversed: Vec<Point> = hole.into_iter().rev().collect();
        let again = Solid::extrude(&rectangle(2.0, 3.0), &[reversed], 0.2).unwrap();
        assert!(again.is_manifold() && close(again.volume().unwrap(), 0.9));
    }

    #[test]
    fn pushing_backwards_still_encloses_a_positive_volume() {
        let solid = Solid::extrude(&rectangle(1.0, 1.0), &[], -2.0).unwrap();
        assert!(solid.is_manifold());
        assert!(close(solid.volume().unwrap(), 2.0));
        assert!(solid.vertices.iter().all(|point| point.z <= 1e-12));
    }

    #[test]
    fn a_concave_profile_keeps_its_area() {
        let ell = vec![
            Point::new(0.0, 0.0, 0.0), Point::new(3.0, 0.0, 0.0), Point::new(3.0, 1.0, 0.0),
            Point::new(1.0, 1.0, 0.0), Point::new(1.0, 3.0, 0.0), Point::new(0.0, 3.0, 0.0),
        ];
        let solid = Solid::extrude(&ell, &[], 1.0).unwrap();
        assert!(solid.is_manifold());
        assert!(close(solid.volume().unwrap(), 5.0));
    }

    #[test]
    fn faces_point_outwards() {
        let solid = Solid::extrude(&rectangle(1.0, 1.0), &[], 1.0).unwrap();
        let centre = Point::new(0.5, 0.5, 0.5);
        for face in &solid.faces {
            let on_face = solid.vertices[face.outer[0]];
            assert!(solid.face_normal(face).dot(on_face - centre) > 0.0);
        }
    }

    #[test]
    fn impossible_shapes_are_refused() {
        assert_eq!(Solid::extrude(&rectangle(1.0, 1.0), &[], 0.0).unwrap_err(), GeometryError::ZeroDistance);
        assert_eq!(Solid::extrude(&rectangle(1.0, 1.0)[..2], &[], 1.0).unwrap_err(), GeometryError::DegenerateLoop);
        let line = vec![Point::new(0.0, 0.0, 0.0), Point::new(1.0, 0.0, 0.0), Point::new(2.0, 0.0, 0.0)];
        assert_eq!(Solid::extrude(&line, &[], 1.0).unwrap_err(), GeometryError::DegenerateLoop);
        let mut bent = rectangle(1.0, 1.0);
        bent[2].z = 0.4;
        assert_eq!(Solid::extrude(&bent, &[], 1.0).unwrap_err(), GeometryError::NotPlanar);
    }

    #[test]
    fn an_open_set_of_faces_is_not_a_solid() {
        let mut solid = Solid::extrude(&rectangle(1.0, 1.0), &[], 1.0).unwrap();
        solid.faces.pop();
        assert!(!solid.is_manifold());
    }

    #[test]
    fn the_mesh_has_flat_faces_and_every_edge_once() {
        let mesh = Solid::extrude(&rectangle(1.0, 1.0), &[], 1.0).unwrap().mesh().unwrap();
        assert_eq!(mesh.positions.len(), 12 * 9);
        assert_eq!(mesh.normals.len(), mesh.positions.len());
        assert_eq!(mesh.edges.len(), 12 * 6);
        assert!(mesh.positions.iter().chain(&mesh.normals).chain(&mesh.edges).all(|value| value.is_finite()));
    }
}
