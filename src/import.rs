//! Importing a model into the open scene: the new model's nodes become a collection inside the scene,
//! its geometry is appended to the scene's mesh, and when both have textures the two atlases are
//! stacked into one (the new model's texture coordinates are remapped to its part).
use crate::atlas::Atlas;
use crate::obj::{Kind, Mesh, Node, FLOATS};

/// Where a mesh's texture lives inside a combined atlas. Texture coordinates are NOT changed: faces
/// without a material rectangle get one covering their atlas part, so they sample exactly the same
/// texels as before (and keep the wrap-around of tiling coordinates inside their own part).
#[derive(Clone, Copy)]
struct Part {
    /// First row of the part, in texels.
    row: f32,
    w: f32,
    h: f32,
}

/// The two atlases stacked (a missing one counts as the default checker atlas).
fn stack(a: Option<&Atlas>, b: Option<&Atlas>) -> (Option<Atlas>, Option<Part>, Option<Part>) {
    if a.is_none() && b.is_none() {
        return (None, None, None);
    }
    let ra = a.cloned().unwrap_or_else(Atlas::procedural);
    let rb = b.cloned().unwrap_or_else(Atlas::procedural);
    let (w, h) = (ra.w.max(rb.w), ra.h + rb.h);
    let mut rgba = vec![0u8; w * h * 4];
    for (part, y0) in [(&ra, 0), (&rb, ra.h)] {
        for y in 0..part.h {
            let dst = ((y0 + y) * w) * 4;
            rgba[dst..dst + part.w * 4].copy_from_slice(&part.rgba[y * part.w * 4..(y + 1) * part.w * 4]);
        }
    }
    let pa = Part { row: 0.0, w: ra.w as f32, h: ra.h as f32 };
    let pb = Part { row: ra.h as f32, w: rb.w as f32, h: rb.h as f32 };
    (Some(Atlas { w, h, rgba }), Some(pa), Some(pb))
}

fn copy_vertices(out: &mut Vec<f32>, src: &[f32], node_off: usize, part: Option<Part>) {
    for v in src.chunks(FLOATS) {
        let mut v = v.to_vec();
        if let Some(p) = part {
            if v[10] > 0.0 {
                v[9] += p.row; // a material rectangle: only its row moves
            } else {
                (v[8], v[9], v[10], v[11]) = (0.0, p.row, p.w, p.h);
            }
        }
        v[12] += node_off as f32;
        out.extend_from_slice(&v);
    }
}

fn union_sphere(c1: [f32; 3], r1: f32, c2: [f32; 3], r2: f32) -> ([f32; 3], f32) {
    let d = ((c2[0] - c1[0]).powi(2) + (c2[1] - c1[1]).powi(2) + (c2[2] - c1[2]).powi(2)).sqrt();
    if r2 <= 0.0 || d + r2 <= r1 {
        return (c1, r1);
    }
    if r1 <= 0.0 || d + r1 <= r2 {
        return (c2, r2);
    }
    let r = (d + r1 + r2) * 0.5;
    let t = (r - r1) / d;
    ([c1[0] + (c2[0] - c1[0]) * t, c1[1] + (c2[1] - c1[1]) * t, c1[2] + (c2[2] - c1[2]) * t], r)
}

/// Appends model `b` to scene `a`. `label` names the collection that holds `b`'s nodes. Names that
/// are already taken get a numeric suffix (edits and scripts are keyed by name).
pub fn merge(a: &Mesh, a_atlas: Option<&Atlas>, b: &Mesh, b_atlas: Option<&Atlas>, label: &str) -> (Mesh, Option<Atlas>) {
    let (atlas, pa, pb) = stack(a_atlas, b_atlas);
    let (node_off, vert_off) = (a.nodes.len(), (a.data.len() / FLOATS) as u32);

    let mut taken: Vec<String> = a.nodes.iter().map(|n| n.name.clone()).collect();
    taken.push(crate::scene::ENV_NODE.to_string());
    taken.push(crate::scene::PRIMS_COLLECTION.to_string());
    let mut nodes = a.nodes.clone();
    for (i, n) in b.nodes.iter().enumerate() {
        let wanted = if i == 0 { label } else { n.name.as_str() };
        let name = crate::scene::unique_name(wanted, |s| taken.iter().any(|t| t == s));
        taken.push(name.clone());
        let parent = if i == 0 { Some(0) } else { n.parent.map(|p| p + node_off) };
        let mut node = Node::new(if i == 0 { Kind::Collection } else { n.kind }, parent, &name, n.hidden);
        (node.center, node.radius, node.tris, node.quads) = (n.center, n.radius, n.tris, n.quads);
        nodes.push(node);
    }
    nodes[0].tris += b.nodes[0].tris;
    nodes[0].quads += b.nodes[0].quads;

    let mut data = Vec::with_capacity(a.data.len() + b.data.len());
    copy_vertices(&mut data, &a.data, 0, pa);
    copy_vertices(&mut data, &b.data, node_off, pb);

    let shift = |v: &[u32]| v.iter().map(|&i| i + vert_off).collect::<Vec<u32>>();
    let mut indices = a.indices[..a.opaque_count].to_vec();
    indices.extend(shift(&b.indices[..b.opaque_count]));
    let opaque_count = indices.len();
    indices.extend_from_slice(&a.indices[a.opaque_count..]);
    indices.extend(shift(&b.indices[b.opaque_count..]));
    let mut wire = a.wire.clone();
    wire.extend(shift(&b.wire));

    let (center, radius) = union_sphere(a.center, a.radius, b.center, b.radius);
    (Mesh { nodes, data, indices, opaque_count, wire, center, radius }, atlas)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(name: &str) -> Mesh {
        crate::obj::parse(&format!("o {name}\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nf 1/1 2/2 3/3 4/4\n")).unwrap()
    }

    #[test]
    fn nodes_are_appended_under_a_new_collection_with_unique_names() {
        let (a, b) = (cube("Box"), cube("Box"));
        let (m, atlas) = merge(&a, None, &b, None, "Box");
        assert!(atlas.is_none());
        assert_eq!(m.nodes.len(), a.nodes.len() + b.nodes.len());
        let names: Vec<&str> = m.nodes.iter().map(|n| n.name.as_str()).collect();
        for (i, n) in names.iter().enumerate() {
            assert!(!names[..i].contains(n), "duplicate name {n}: {names:?}");
        }
        let added = a.nodes.len();
        assert_eq!(m.nodes[added].kind, Kind::Collection);
        assert_eq!(m.nodes[added].parent, Some(0));
        assert!(m.nodes[added + 1..].iter().all(|n| n.parent.is_some_and(|p| p >= added)));
        let verts = m.data.len() / FLOATS;
        assert!(m.indices.iter().chain(&m.wire).all(|&i| (i as usize) < verts));
        assert!(m.data.chunks(FLOATS).all(|v| (v[12] as usize) < m.nodes.len()));
        assert!(m.data.chunks(FLOATS).skip(verts / 2).all(|v| v[12] as usize >= added), "the new vertices point at the new nodes");
    }

    #[test]
    fn two_textured_models_share_a_stacked_atlas() {
        let a = Atlas { w: 4, h: 2, rgba: vec![10; 4 * 2 * 4] };
        let b = Atlas { w: 2, h: 2, rgba: vec![200; 2 * 2 * 4] };
        let (m, atlas) = merge(&cube("A"), Some(&a), &cube("B"), Some(&b), "B");
        let atlas = atlas.unwrap();
        assert_eq!((atlas.w, atlas.h), (4, 4));
        assert_eq!(atlas.rgba[0], 10);
        assert_eq!(atlas.rgba[(3 * 4) * 4], 200, "the second atlas starts at row 2");
        let verts = m.data.len() / FLOATS;
        // texture coordinates are untouched ...
        let orig: Vec<[f32; 2]> = cube("A").data.chunks(FLOATS).map(|v| [v[3], v[4]]).collect();
        for (i, v) in m.data.chunks(FLOATS).enumerate() {
            assert_eq!([v[3], v[4]], orig[i % orig.len()], "vertex {i}");
        }
        // ... and each model's faces point at their own part of the atlas
        let rects: Vec<[f32; 4]> = m.data.chunks(FLOATS).map(|v| [v[8], v[9], v[10], v[11]]).collect();
        assert!(rects[..verts / 2].iter().all(|r| *r == [0.0, 0.0, 4.0, 2.0]));
        assert!(rects[verts / 2..].iter().all(|r| *r == [0.0, 2.0, 2.0, 2.0]));
    }

    #[test]
    fn material_rectangles_move_with_their_part_and_plain_models_stay_exact() {
        let (plain, _) = merge(&cube("A"), None, &cube("B"), None, "B");
        assert!(plain.data.chunks(FLOATS).all(|v| v[10] == 0.0), "no atlas: nothing is rewritten");
        let mut b = cube("B");
        for v in b.data.chunks_mut(FLOATS) {
            (v[8], v[9], v[10], v[11]) = (1.0, 1.0, 2.0, 2.0);
        }
        let (m, _) = merge(&cube("A"), Some(&Atlas { w: 4, h: 4, rgba: vec![0; 64] }), &b, Some(&Atlas { w: 4, h: 4, rgba: vec![0; 64] }), "B");
        let last = m.data.chunks(FLOATS).last().unwrap();
        assert_eq!([last[8], last[9], last[10], last[11]], [1.0, 5.0, 2.0, 2.0]);
    }

    #[test]
    fn bounding_sphere_covers_both() {
        let (c, r) = union_sphere([0.0; 3], 1.0, [10.0, 0.0, 0.0], 1.0);
        assert!((r - 6.0).abs() < 1e-5 && (c[0] - 5.0).abs() < 1e-5);
        assert_eq!(union_sphere([0.0; 3], 5.0, [1.0, 0.0, 0.0], 1.0).1, 5.0);
    }
}
