//! A single room with a pitched roof, built from pushed faces, for explaining how wind moves
//! through a building. Every part is a closed solid; openings are real holes through the walls.
//!
//! Axes: x across the room, y up, z through it. The inlet wall faces +z.

use serde::{Deserialize, Serialize};

use crate::{loop_normal, GeometryError, Mesh, Point, Solid};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(default)]
pub struct Opening {
    pub width: f64,
    /// Height of the bottom of the opening above the floor.
    pub sill: f64,
    /// Height of the top of the opening above the floor.
    pub head: f64,
}

impl Default for Opening {
    fn default() -> Self {
        Self { width: 3.0, sill: 1.0, head: 2.2 }
    }
}

/// Dimensions in metres. They describe a schematic, not a surveyed building.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(default)]
pub struct RoomSpec {
    pub width: f64,
    pub depth: f64,
    pub height: f64,
    pub wall: f64,
    pub inlet: Opening,
    pub outlet: Opening,
    pub roof_pitch_degrees: f64,
    pub overhang: f64,
    pub roof_thickness: f64,
}

impl Default for RoomSpec {
    fn default() -> Self {
        Self {
            width: 6.0,
            depth: 4.0,
            height: 3.0,
            wall: 0.2,
            inlet: Opening::default(),
            outlet: Opening { width: 2.0, sill: 1.8, head: 2.6 },
            roof_pitch_degrees: 24.0,
            overhang: 0.5,
            roof_thickness: 0.14,
        }
    }
}

/// One nameable piece of the building, ready to draw.
#[derive(Clone, Debug, Serialize)]
pub struct Part {
    /// What the piece is: `inlet`, `outlet`, `wall`, `floor`, `roof` or `vent`.
    pub id: &'static str,
    pub mesh: Mesh,
    /// Where the piece moves to when the building is taken apart.
    pub apart: [f32; 3],
    /// Checked from topology, so a drawing never shows a part with gaps in it.
    pub volume: f64,
}

fn part(id: &'static str, solid: Solid, apart: [f32; 3]) -> Result<Part, GeometryError> {
    debug_assert!(solid.is_manifold(), "{id} is not a closed solid");
    Ok(Part { id, mesh: solid.mesh()?, apart, volume: solid.volume()? })
}

/// The loop wound so that its normal points along `direction`, which is the way it will be pushed.
fn facing(mut points: Vec<Point>, direction: Point) -> Vec<Point> {
    if loop_normal(&points).dot(direction) < 0.0 {
        points.reverse();
    }
    points
}

/// A wall in the plane z = `z`, pushed `thickness` along `direction` (+1 or -1 in z), with an opening.
fn wall_with_opening(spec: &RoomSpec, z: f64, direction: f64, opening: Opening) -> Result<Solid, GeometryError> {
    let half = spec.width / 2.0;
    let outline = facing(
        vec![Point::new(-half, 0.0, z), Point::new(half, 0.0, z), Point::new(half, spec.height, z), Point::new(-half, spec.height, z)],
        Point::new(0.0, 0.0, direction),
    );
    let reach = opening.width / 2.0;
    let hole = vec![
        Point::new(-reach, opening.sill, z), Point::new(reach, opening.sill, z),
        Point::new(reach, opening.head, z), Point::new(-reach, opening.head, z),
    ];
    Solid::extrude(&outline, &[hole], spec.wall)
}

pub fn build(spec: &RoomSpec) -> Result<Vec<Part>, GeometryError> {
    let (half_width, half_depth) = (spec.width / 2.0, spec.depth / 2.0);
    let rise = half_depth * spec.roof_pitch_degrees.to_radians().tan();
    let ridge = spec.height + rise;
    let mut parts = Vec::new();

    // Floor slab, pushed upwards.
    let floor = [
        Point::new(-half_width, -0.15, half_depth), Point::new(half_width, -0.15, half_depth),
        Point::new(half_width, -0.15, -half_depth), Point::new(-half_width, -0.15, -half_depth),
    ];
    parts.push(part("floor", Solid::extrude(&floor, &[], 0.15)?, [0.0, 0.0, 0.0])?);

    // Inlet wall with a low, wide opening; outlet wall with a higher one.
    parts.push(part("inlet", wall_with_opening(spec, half_depth - spec.wall, 1.0, spec.inlet)?, [0.0, 0.0, 1.4])?);
    parts.push(part("outlet", wall_with_opening(spec, -half_depth + spec.wall, -1.0, spec.outlet)?, [0.0, 0.0, -1.4])?);

    // Gable end walls, set between the other two and rising to meet the underside of the roof.
    let inner = half_depth - spec.wall;
    let shoulder = ridge - rise * inner / half_depth;
    for side in [-1.0, 1.0] {
        let x = side * half_width - if side > 0.0 { spec.wall } else { 0.0 };
        let gable = facing(
            vec![
                Point::new(x, 0.0, inner), Point::new(x, 0.0, -inner), Point::new(x, shoulder, -inner),
                Point::new(x, ridge, 0.0), Point::new(x, shoulder, inner),
            ],
            Point::new(1.0, 0.0, 0.0),
        );
        parts.push(part("wall", Solid::extrude(&gable, &[], spec.wall)?, [side as f32 * 1.4, 0.0, 0.0])?);
    }

    // Two roof slabs: a thin sloping cross-section pushed along the ridge.
    let eave = half_depth + spec.overhang;
    let drop = eave * spec.roof_pitch_degrees.to_radians().tan();
    let start = -(half_width + spec.overhang);
    for side in [1.0, -1.0] {
        let section = facing(
            vec![
                Point::new(start, ridge, 0.0), Point::new(start, ridge - drop, side * eave),
                Point::new(start, ridge - drop + spec.roof_thickness, side * eave), Point::new(start, ridge + spec.roof_thickness, 0.0),
            ],
            Point::new(1.0, 0.0, 0.0),
        );
        parts.push(part("roof", Solid::extrude(&section, &[], spec.width + 2.0 * spec.overhang)?, [0.0, 2.2, 0.0])?);
    }

    // Ridge vent: a small box on top.
    let top = ridge + spec.roof_thickness;
    let vent = [
        Point::new(-1.2, top, 0.25), Point::new(1.2, top, 0.25), Point::new(1.2, top, -0.25), Point::new(-1.2, top, -0.25),
    ];
    parts.push(part("vent", Solid::extrude(&vent, &[], 0.3)?, [0.0, 2.9, 0.0])?);
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_part_is_a_closed_solid_with_a_drawable_mesh() {
        let parts = build(&RoomSpec::default()).unwrap();
        let ids: Vec<&str> = parts.iter().map(|part| part.id).collect();
        assert_eq!(ids, ["floor", "inlet", "outlet", "wall", "wall", "roof", "roof", "vent"]);
        for part in &parts {
            assert!(part.volume > 0.0, "{} encloses nothing", part.id);
            assert!(!part.mesh.positions.is_empty() && part.mesh.positions.len() % 9 == 0);
            assert_eq!(part.mesh.positions.len(), part.mesh.normals.len());
            assert!(part.mesh.positions.iter().all(|value| value.is_finite()));
        }
    }

    #[test]
    fn openings_are_real_holes_through_the_walls() {
        let spec = RoomSpec::default();
        let parts = build(&spec).unwrap();
        let volume = |id: &str| parts.iter().find(|part| part.id == id).unwrap().volume;
        let solid_wall = spec.width * spec.height * spec.wall;
        let hole = |opening: Opening| opening.width * (opening.head - opening.sill) * spec.wall;
        assert!((volume("inlet") - (solid_wall - hole(spec.inlet))).abs() < 1e-9);
        assert!((volume("outlet") - (solid_wall - hole(spec.outlet))).abs() < 1e-9);
    }

    #[test]
    fn the_room_follows_its_dimensions() {
        let wide = build(&RoomSpec { width: 9.0, ..RoomSpec::default() }).unwrap();
        let reach = wide.iter().flat_map(|part| part.mesh.positions.chunks(3)).map(|point| point[0].abs()).fold(0.0, f32::max);
        assert!((reach - 5.0).abs() < 1e-5, "the roof should reach half the width plus the overhang");
    }

    #[test]
    fn an_opening_with_no_size_is_refused_rather_than_drawn_wrongly() {
        let spec = RoomSpec { inlet: Opening { width: 0.0, ..Opening::default() }, ..RoomSpec::default() };
        assert!(build(&spec).is_err());
    }
}
