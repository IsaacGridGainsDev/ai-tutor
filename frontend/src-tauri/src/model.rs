//! 3D models built by the geometry engine and handed to the window as meshes.

use tutor_geometry::room::{build, Part, RoomSpec};

/// The schematic room used to explain wind and ventilation. With no dimensions given, the defaults apply.
#[tauri::command]
pub fn room_model(spec: Option<RoomSpec>) -> Result<Vec<Part>, String> {
    build(&spec.unwrap_or_default()).map_err(|error| error.to_string())
}
