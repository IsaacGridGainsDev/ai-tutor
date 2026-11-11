# How SketchUp and Rhino model 3D: evidence and what the tutor takes from it

Targets on this machine: SketchUp 2027 (27.0.264) and Rhino 9 (9.0.26300).
Method: read-only. File inventory, the import tables of the main programs, and the names exported
by the libraries each vendor publishes as its programming interface (`SketchUpAPI.dll`, the
SketchUp C API; `opennurbs.dll` and `rhcommon_c.dll`, Rhino's open-source toolkit and the native
side of RhinoCommon). Nothing was run, disassembled or decompiled, and no model file was opened.
Public documentation was used to interpret the names. Both licence agreements restrict reverse
engineering of the programs themselves, so the inspection stayed with the published interfaces.

## SketchUp: a polygonal boundary model with explicit topology

Observed in `SketchUpAPI.dll` (1,330 exported functions):

| Family | Functions seen | What it shows |
| --- | --- | --- |
| `SUVertex` | `GetPosition`, `GetEdges`, `GetFaces`, `GetLoops` | A vertex knows everything attached to it. |
| `SUEdge` | `GetStartVertex`, `GetEndVertex`, `GetFaces`, `GetSoft`, `GetSmooth`, `ReversedInFace` | Edges are first-class and shared between faces. Soft and smooth are edge flags, not separate geometry. |
| `SUEdgeUse` | `GetEdge`, `GetLoop`, `GetFace`, `GetNext`, `GetPrevious`, `GetPartners`, `IsReversed` | One record per use of an edge by a face loop, linked round the loop and across to the other faces on the same edge. |
| `SULoop` | `GetEdgeUses`, `IsOuterLoop`, `IsConvex`, `GetWinding` | A face boundary is an ordered loop. |
| `SUFace` | `GetOuterLoop`, `GetInnerLoops`, `AddInnerLoop`, `GetPlane`, `GetNormal`, `GetArea`, front and back materials | A face is planar: one outer loop, any number of holes, a plane, two sides. |
| `SUCurve`, `SUArcCurve` | `CreateWithEdges`, `GetRadius`, `GetCenter`, `IsPolygon` | A curve is a chain of straight edges that remembers it is an arc. There is no true curved surface. |
| `SUGeometryInput`, `SULoopInput` | `AddVertex`, `AddFace`, `FaceAddInnerLoop`, `AddEdge` | Geometry is built by describing vertices and loops, then merged into the model in one step. |
| `SUMeshHelper` | `GetNumTriangles`, `GetVertices`, `GetNormals`, `GetVertexIndices` | Triangles are derived from faces for drawing; they are not the model. |
| `SUComponentDefinition`, `SUComponentInstance`, `SUGroup` | `GetEntities`, `CreateInstance`, `GetTransform`, `IsManifold`, `ComputeVolume` | Reuse is a definition plus placed instances with a transform. Solidity and volume are computed from topology. |

`SketchUp.exe` imports `d3d12.dll`, `dxgi.dll` and `opengl32.dll`, and the interface is Qt 6.

Inference: SketchUp is a face-edge-vertex boundary model in which every face is a flat polygon with
holes. Its characteristic look, flat faces with every edge drawn, falls straight out of that model.
Push/Pull is an extrusion of a face's loops along its normal; the API does not export it, so its
exact behaviour (how it merges with neighbouring geometry) is not confirmed here.

## Rhino: NURBS geometry, trimmed into boundary representations

Observed in `opennurbs.dll` (8,192 exports) and `rhcommon_c.dll` (7,617 exports):

- Geometry types: `ON_NurbsCurve`, `ON_BezierCurve`, `ON_Curve`, `ON_NurbsSurface`, `ON_Surface`,
  `ON_Brep` with `ON_BrepFace`, `ON_Extrusion`, `ON_SubD`, `ON_Mesh`, `ON_PointCloud`.
- Supporting types: `ON_Xform` (transforms), `ON_Intersect`, `ON_HiddenLineDrawing`,
  `ON_3dmUnitsAndTolerances`, `ON_MeshIntersectionOptions`, `ON_BinaryArchive` and `ONX_Model`
  (the `.3dm` file).
- `RhinoCore.dll` imports `opennurbs.dll`, `tl.dll`, `linalg.dll`, `d3d11.dll` and `d3d12.dll`.

Inference: Rhino stores exact curved geometry. A solid is a `Brep`: NURBS surfaces trimmed by
curves and stitched together by shared edges, all within a stated tolerance. Meshes are generated
from that for display. `ON_Extrusion` is a lightweight special case for the common
profile-pushed-along-a-line object. openNURBS is published by McNeel as open source for reading
and writing `.3dm` files; McNeel's documentation says it leaves out intersections, booleans and
closest-point calculations, which live in the closed part of Rhino.

## Unknowns

- SketchUp's inference engine, auto-merging ("sticky geometry") and Push/Pull internals.
- Rhino's intersection, boolean and meshing algorithms.
- How either program batches and draws geometry on the GPU.

Confirming these would need disassembly or runtime observation, which was not done.

## What the tutor adopts

The tutor draws buildings and their parts, which are overwhelmingly flat-faced. It therefore
follows SketchUp's model, not Rhino's:

- planar faces with an outer loop and inner loops (holes), sharing vertices and edges
- solids made by pushing a face along its normal, with the holes carried through
- solidity checked from topology (every edge used by exactly two faces) and volume computed from it
- parts as named groups with their own transform
- triangles and edge lines derived for drawing, never stored as the model
- an extrusion kept as the cheap common case, as `ON_Extrusion` is in Rhino

Not adopted: NURBS surfaces and trimmed boundary representations. They matter for curved forms and
would be a much larger undertaking; the Rust crate `truck` (Apache 2.0) is the candidate if the
tutor ever needs them, rather than writing one.

Sources: [SketchUp C API reference](https://extensions.sketchup.com/developers/sketchup_c_api/sketchup/struct_s_u_edge_use_ref.html),
[What is openNURBS](https://developer.rhino3d.com/guides/opennurbs/what-is-opennurbs/),
[openNURBS feature page](https://www.rhino3d.com/features/developer/opennurbs),
[truck](https://github.com/ricosjp/truck).
