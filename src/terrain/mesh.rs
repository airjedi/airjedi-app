//! Terrain mesh generation from heightmap data.
//!
//! Generates subdivided grid meshes with vertex displacement from elevation data.
//! Each tile gets its own mesh with heights baked into vertex positions.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use super::heightmap::HeightmapData;

/// Default depth for skirt geometry (world units below edge vertices).
const SKIRT_DEPTH: f32 = 50.0;

/// Neighbor resolution info for edge stitching.
/// When a neighbor has lower resolution than this tile, edge vertices
/// are snapped to match the coarser grid to prevent T-junction cracks.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct NeighborLod {
    /// Resolution of the neighbor on the top edge (row 0), or None if same/higher.
    pub top: Option<u32>,
    /// Resolution of the neighbor on the bottom edge (last row).
    pub bottom: Option<u32>,
    /// Resolution of the neighbor on the left edge (column 0).
    pub left: Option<u32>,
    /// Resolution of the neighbor on the right edge (last column).
    pub right: Option<u32>,
}

/// Generate a terrain mesh from heightmap data.
///
/// # Parameters
/// - `heightmap`: The decoded elevation data (256x256 grid of meters)
/// - `tile_size`: World-space size of the tile (512.0)
/// - `resolution`: Grid subdivision (e.g., 32 means 32x32 quads, yielding 33x33 vertices)
/// - `altitude_scale`: Combined scale factor (PIXEL_SCALE * ALTITUDE_EXAGGERATION = 400.0).
///   Elevation is converted to world units as: `elevation_m * 0.001 * altitude_scale`.
/// - `add_skirts`: Whether to add skirt geometry for crack prevention between tiles
/// - `neighbor_lod`: Resolution of adjacent tiles for edge stitching
///
/// Returns a Bevy `Mesh` with positions, normals, UVs, and triangle indices.
pub(crate) fn generate_terrain_mesh(
    heightmap: &HeightmapData,
    tile_size: f32,
    resolution: u32,
    altitude_scale: f32,
    add_skirts: bool,
    neighbor_lod: &NeighborLod,
) -> Mesh {
    let verts_per_side = resolution + 1;
    let vertex_count = (verts_per_side * verts_per_side) as usize;

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(vertex_count);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(vertex_count);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(vertex_count);

    let half_size = tile_size / 2.0;
    // Texel size in UV space for normal computation (one heightmap pixel).
    let texel_size = 1.0 / heightmap.width().max(1) as f32;

    // Generate vertices: row-major order (z then x).
    for row in 0..verts_per_side {
        for col in 0..verts_per_side {
            // UV coordinates: (0,0) at top-left, (1,1) at bottom-right.
            let u = col as f32 / resolution as f32;
            let v = row as f32 / resolution as f32;

            // World-space position: centered at origin, X = east, Z = south.
            let x = -half_size + u * tile_size;
            let z = -half_size + v * tile_size;

            // Sample elevation and convert meters -> world units.
            let elevation_m = sample_heightmap(heightmap, u, v);
            let y = elevation_m * 0.001 * altitude_scale;

            positions.push([x, y, z]);
            uvs.push([u, v]);

            // Compute normal via central finite differences.
            let normal = compute_normal(heightmap, u, v, texel_size, altitude_scale);
            normals.push(normal.into());
        }
    }

    // Edge stitching: snap edge vertices to match lower-resolution neighbors.
    // For each edge where the neighbor has fewer subdivisions, intermediate
    // vertices are linearly interpolated between the coarser grid points.
    // This eliminates T-junction cracks at LOD boundaries.
    stitch_edge_vertices(&mut positions, resolution, verts_per_side, neighbor_lod);

    // Generate triangle indices: two triangles per grid cell.
    let quad_count = (resolution * resolution) as usize;
    let mut indices: Vec<u32> = Vec::with_capacity(quad_count * 6);

    for row in 0..resolution {
        for col in 0..resolution {
            let top_left = row * verts_per_side + col;
            let top_right = top_left + 1;
            let bottom_left = top_left + verts_per_side;
            let bottom_right = bottom_left + 1;

            // First triangle (upper-left).
            indices.push(top_left);
            indices.push(bottom_left);
            indices.push(top_right);

            // Second triangle (lower-right).
            indices.push(top_right);
            indices.push(bottom_left);
            indices.push(bottom_right);
        }
    }

    // Optionally add skirt geometry to hide cracks between tiles at different LODs.
    if add_skirts {
        add_skirt_geometry(
            &mut positions,
            &mut normals,
            &mut uvs,
            &mut indices,
            resolution,
            SKIRT_DEPTH,
        );
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Sample the heightmap at a normalized UV coordinate (0.0 to 1.0).
/// Uses bilinear interpolation for smooth terrain between heightmap pixels.
fn sample_heightmap(heightmap: &HeightmapData, u: f32, v: f32) -> f32 {
    let w = heightmap.width() as f32;
    let h = heightmap.height() as f32;

    // Map UV to pixel coordinates (clamped to valid range).
    let px = (u * (w - 1.0)).clamp(0.0, w - 1.0);
    let py = (v * (h - 1.0)).clamp(0.0, h - 1.0);

    let x0 = px.floor() as usize;
    let y0 = py.floor() as usize;
    let x1 = (x0 + 1).min(heightmap.width() - 1);
    let y1 = (y0 + 1).min(heightmap.height() - 1);

    let fx = px - px.floor();
    let fy = py - py.floor();

    // Fetch the four neighboring elevation samples.
    let e00 = heightmap.elevation(x0, y0);
    let e10 = heightmap.elevation(x1, y0);
    let e01 = heightmap.elevation(x0, y1);
    let e11 = heightmap.elevation(x1, y1);

    // Bilinear interpolation.
    let top = e00 * (1.0 - fx) + e10 * fx;
    let bottom = e01 * (1.0 - fx) + e11 * fx;
    top * (1.0 - fy) + bottom * fy
}

/// Compute the surface normal at a grid point using central finite differences.
/// Samples neighboring heights to determine the slope in both X and Z directions,
/// then returns the normalized cross product.
fn compute_normal(
    heightmap: &HeightmapData,
    u: f32,
    v: f32,
    texel_size: f32,
    altitude_scale: f32,
) -> Vec3 {
    // Sample heights at neighboring positions.
    let h_left = sample_heightmap(heightmap, (u - texel_size).max(0.0), v);
    let h_right = sample_heightmap(heightmap, (u + texel_size).min(1.0), v);
    let h_down = sample_heightmap(heightmap, u, (v - texel_size).max(0.0));
    let h_up = sample_heightmap(heightmap, u, (v + texel_size).min(1.0));

    // Convert elevation differences to world units.
    let scale = 0.001 * altitude_scale;
    let dh_dx = (h_right - h_left) * scale;
    let dh_dz = (h_up - h_down) * scale;

    // The step distance in world space that corresponds to one texel.
    // We use 2 * texel_size because central differences span two texels,
    // but the actual world distance depends on the tile size. Since we are
    // computing in a normalised UV space where the full tile maps to 1.0,
    // and the tile size is uniform, the ratio cancels — we only need the
    // relative slope. We construct the normal from the tangent vectors:
    //   tangent_x = (2*step_world, dh_dx, 0)
    //   tangent_z = (0, dh_dz, 2*step_world)
    // Normal = tangent_x × tangent_z  (Y-up cross product).
    //
    // With step = 2 * texel_size (UV) mapped to world units, the magnitudes
    // cancel when we normalise, so we can simplify:
    let normal = Vec3::new(-dh_dx, 2.0 * texel_size, -dh_dz).normalize_or(Vec3::Y);
    normal
}

/// Snap edge vertices to match a lower-resolution neighbor's grid.
///
/// When a neighbor tile has fewer subdivisions (e.g., 16 vs our 64), its edge
/// has fewer vertices. Our edge vertices between the coarser grid points would
/// not align, creating T-junction cracks. This function linearly interpolates
/// the Y (height) of intermediate vertices between the coarser grid points so
/// both tiles produce identical edge geometry.
fn stitch_edge_vertices(
    positions: &mut [[f32; 3]],
    resolution: u32,
    verts_per_side: u32,
    neighbor_lod: &NeighborLod,
) {
    // For a given edge, `our_count` = resolution + 1 vertices.
    // The neighbor has `neighbor_res + 1` vertices along the same edge.
    // We only stitch when the neighbor is strictly coarser (lower resolution).
    let stitch_edge = |positions: &mut [[f32; 3]], edge_indices: &[u32], neighbor_res: u32| {
        if neighbor_res >= resolution || neighbor_res == 0 || resolution % neighbor_res != 0 {
            return; // Same/higher res or not evenly divisible — skip.
        }

        // The coarser grid divides the edge into `neighbor_res` segments.
        // Our grid divides it into `resolution` segments.
        // The ratio tells us how many of our vertices fall between each
        // pair of coarse grid points.
        let step = resolution / neighbor_res; // e.g., 64/16 = 4

        for coarse_seg in 0..neighbor_res {
            let start_idx = (coarse_seg * step) as usize;
            let end_idx = ((coarse_seg + 1) * step) as usize;

            let start_y = positions[edge_indices[start_idx] as usize][1];
            let end_y = positions[edge_indices[end_idx] as usize][1];

            // Interpolate intermediate vertices
            for k in 1..step as usize {
                let t = k as f32 / step as f32;
                let idx = edge_indices[start_idx + k] as usize;
                positions[idx][1] = start_y * (1.0 - t) + end_y * t;
            }
        }
    };

    // Top edge: row 0, columns 0..=resolution
    if let Some(neighbor_res) = neighbor_lod.top {
        let edge: Vec<u32> = (0..verts_per_side).collect();
        stitch_edge(positions, &edge, neighbor_res);
    }

    // Bottom edge: last row
    if let Some(neighbor_res) = neighbor_lod.bottom {
        let edge: Vec<u32> = (0..verts_per_side)
            .map(|col| resolution * verts_per_side + col)
            .collect();
        stitch_edge(positions, &edge, neighbor_res);
    }

    // Left edge: column 0
    if let Some(neighbor_res) = neighbor_lod.left {
        let edge: Vec<u32> = (0..verts_per_side)
            .map(|row| row * verts_per_side)
            .collect();
        stitch_edge(positions, &edge, neighbor_res);
    }

    // Right edge: last column
    if let Some(neighbor_res) = neighbor_lod.right {
        let edge: Vec<u32> = (0..verts_per_side)
            .map(|row| row * verts_per_side + resolution)
            .collect();
        stitch_edge(positions, &edge, neighbor_res);
    }
}

/// Add skirt geometry along the mesh edges to prevent cracks between
/// adjacent tiles at different LOD levels. Skirts extend downward by
/// a fixed amount from each edge vertex.
///
/// For each of the four edges (top, bottom, left, right), we duplicate the
/// edge vertices with Y offset by `-skirt_depth` and create triangles
/// connecting the original edge vertices to their lowered counterparts.
fn add_skirt_geometry(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
    resolution: u32,
    skirt_depth: f32,
) {
    let verts_per_side = resolution + 1;

    // Helper: given a sequence of edge vertex indices, create a skirt strip.
    // `edge_indices` lists original vertex indices along one edge in order.
    let mut add_edge_skirt = |edge_indices: Vec<u32>| {
        let base = positions.len() as u32;
        // Duplicate edge vertices, shifted downward.
        for (i, &orig_idx) in edge_indices.iter().enumerate() {
            let orig = positions[orig_idx as usize];
            positions.push([orig[0], orig[1] - skirt_depth, orig[2]]);
            // Skirt normals point in the same direction as the edge vertex.
            normals.push(normals[orig_idx as usize]);
            uvs.push(uvs[orig_idx as usize]);

            // Create two triangles connecting this edge segment to the skirt.
            if i > 0 {
                let prev_edge = edge_indices[i - 1];
                let curr_edge = orig_idx;
                let prev_skirt = base + (i as u32 - 1);
                let curr_skirt = base + i as u32;

                // Triangle 1.
                indices.push(prev_edge);
                indices.push(prev_skirt);
                indices.push(curr_edge);

                // Triangle 2.
                indices.push(curr_edge);
                indices.push(prev_skirt);
                indices.push(curr_skirt);
            }
        }
    };

    // Top edge: row 0, columns 0..resolution (left to right).
    let top_edge: Vec<u32> = (0..verts_per_side).collect();
    add_edge_skirt(top_edge);

    // Bottom edge: last row, columns 0..resolution (left to right).
    let bottom_edge: Vec<u32> = (0..verts_per_side)
        .map(|col| resolution * verts_per_side + col)
        .collect();
    add_edge_skirt(bottom_edge);

    // Left edge: column 0, rows 0..resolution (top to bottom).
    let left_edge: Vec<u32> = (0..verts_per_side)
        .map(|row| row * verts_per_side)
        .collect();
    add_edge_skirt(left_edge);

    // Right edge: last column, rows 0..resolution (top to bottom).
    let right_edge: Vec<u32> = (0..verts_per_side)
        .map(|row| row * verts_per_side + resolution)
        .collect();
    add_edge_skirt(right_edge);
}

/// Choose mesh resolution based on distance from camera.
/// Near tiles get 64x64, mid tiles 32x32, far tiles 16x16.
///
/// `zoom_offset` is how many zoom levels below the camera's current zoom
/// this tile sits: 0 = same zoom (nearest), 1 = one level below, 2+ = far.
pub(crate) fn resolution_for_zoom_offset(zoom_offset: u32) -> u32 {
    match zoom_offset {
        0 => 64,
        1 => 32,
        _ => 16,
    }
}
