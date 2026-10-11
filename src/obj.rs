use crate::math::{self, Vec3};

/// Floats per vertex: `pos(3) uv(2) face_normal(3) atlas_rect(4) node_id(1)`.
pub const FLOATS: usize = 13;

/// A polygon: (position, uv) per corner, counter-clockwise seen from outside.
pub type Face = Vec<(Vec3, [f32; 2])>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Collection,
    Object,
    /// The scene's environment settings (fog, sky); has no geometry.
    Environment,
    /// One material of an object (a Blender material slot). Its faces belong to this node, which is a child of
    /// the object and follows its transform; named "<object>::<material>".
    Material,
}

/// One entry of the scene hierarchy shown in the explorer.
#[derive(Clone, Debug)]
pub struct Node {
    pub kind: Kind,
    pub parent: Option<usize>,
    pub name: String,
    pub hidden: bool,
    /// Bounding sphere of the node's geometry (including children); radius 0 = no geometry.
    /// For objects the centre is also the pivot that transforms act around.
    pub center: Vec3,
    pub radius: f32,
    pub tris: usize,
    pub quads: usize,
}

impl Node {
    /// The material's own name (what edits are keyed by), for material nodes.
    pub fn material_name(&self) -> Option<&str> {
        (self.kind == Kind::Material).then(|| self.name.split_once("::").map_or(self.name.as_str(), |(_, m)| m))
    }

    pub fn new(kind: Kind, parent: Option<usize>, name: &str, hidden: bool) -> Self {
        Node { kind, parent, name: name.to_string(), hidden, center: [0.0; 3], radius: 0.0, tris: 0, quads: 0 }
    }
}

/// Indexed mesh. Triangles and quads are kept as primitives (a quad is drawn as two triangles
/// sharing its 4 vertices); faces with more than 4 vertices are fan-triangulated.
///
/// `indices` holds the opaque faces first, then the blended (translucent) ones.
/// `wire` is a line list of the real polygon edges (quads show 4 edges, no diagonal).
///
/// Besides plain OBJ, extension lines (written by the Blender exporter) are understood:
///   `usemtl blend`          -> following faces go to the blended pass (any other name = opaque)
///   `mtlrect x y w h`       -> atlas rectangle in pixels; UVs wrap (tile) inside it.
///   `gpr_node <collection|object> <parent_index> <hidden 0|1> <name>` declares a hierarchy node
///   `gpr_use <index>`       -> following faces belong to that node
/// Without a rect, UVs are clamped over the whole atlas. In plain OBJ files each `o`/`g` line
/// becomes an object under the root collection (`nodes[0]`).
impl Mesh {
    /// The object a vertex node id belongs to (a material node belongs to its object).
    pub fn object_of(&self, id: usize) -> usize {
        match self.nodes.get(id) {
            Some(n) if n.kind == Kind::Material => n.parent.unwrap_or(id),
            _ => id,
        }
    }

    /// Per node: the lowest and highest texture coordinate of its faces and 1 / the range, for "stretch".
    pub fn uv_boxes(&self) -> Vec<[f32; 4]> {
        let mut lo = vec![[f32::MAX; 2]; self.nodes.len()];
        let mut hi = vec![[f32::MIN; 2]; self.nodes.len()];
        for v in self.data.chunks_exact(FLOATS) {
            let id = v[FLOATS - 1] as usize;
            if id < lo.len() {
                for k in 0..2 {
                    lo[id][k] = lo[id][k].min(v[3 + k]);
                    hi[id][k] = hi[id][k].max(v[3 + k]);
                }
            }
        }
        (0..self.nodes.len())
            .map(|i| {
                if lo[i][0] > hi[i][0] {
                    return [0.0, 0.0, 1.0, 1.0];
                }
                let r = |k: usize| 1.0 / (hi[i][k] - lo[i][k]).max(1e-4);
                [lo[i][0], lo[i][1], r(0), r(1)]
            })
            .collect()
    }
}

pub struct Mesh {
    pub nodes: Vec<Node>,
    pub data: Vec<f32>,
    pub indices: Vec<u32>,
    pub opaque_count: usize,
    pub wire: Vec<u32>,
    pub center: Vec3,
    pub radius: f32,
}

/// Newell's method: a robust polygon normal even for non-planar or degenerate quads.
fn face_normal(face: &[(Vec3, [f32; 2])]) -> Vec3 {
    let mut n = [0.0f32; 3];
    for i in 0..face.len() {
        let (a, b) = (face[i].0, face[(i + 1) % face.len()].0);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    math::normalize(n)
}

/// Appends a polygon: vertices into `data`, fan triangles into `tris`, outline into `wire`.
pub fn push_face(data: &mut Vec<f32>, tris: &mut Vec<u32>, wire: &mut Vec<u32>, face: &[(Vec3, [f32; 2])], rect: [f32; 4], node: usize) {
    let base = (data.len() / FLOATS) as u32;
    let n = face_normal(face);
    for (p, uv) in face {
        data.extend_from_slice(&[p[0], p[1], p[2], uv[0], uv[1], n[0], n[1], n[2]]);
        data.extend_from_slice(&rect);
        data.push(node as f32);
    }
    let len = face.len() as u32;
    for i in 1..len - 1 {
        tris.extend_from_slice(&[base, base + i, base + i + 1]);
    }
    for i in 0..len {
        wire.extend_from_slice(&[base + i, base + (i + 1) % len]);
    }
}

pub fn load(path: &str) -> Result<Mesh, String> {
    parse(&std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?)
}

type Bounds = ([f32; 3], [f32; 3]);
const EMPTY: Bounds = ([f32::MAX; 3], [f32::MIN; 3]);

pub fn parse(text: &str) -> Result<Mesh, String> {
    let mut pos: Vec<Vec3> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut data: Vec<f32> = Vec::new();
    let (mut opaque, mut blend, mut wire): (Vec<u32>, Vec<u32>, Vec<u32>) = (Vec::new(), Vec::new(), Vec::new());
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    let mut rect = [0.0f32; 4];
    let mut in_blend = false;
    let mut nodes = vec![Node::new(Kind::Collection, None, "Scene", false)];
    let mut bounds: Vec<Bounds> = vec![EMPTY];
    let mut current: Option<usize> = None;
    // the material the following faces use, and the node made for each (object, material) pair
    let mut material: Option<String> = None;
    let mut mat_nodes: std::collections::HashMap<(usize, String), usize> = std::collections::HashMap::new();

    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut it = line.split_whitespace();
        let err = |m: &str| format!("line {}: {m}", n + 1);
        let nums = |it: std::str::SplitWhitespace, k: usize| -> Result<Vec<f32>, String> {
            let v: Vec<f32> = it.take(k).map(|s| s.parse().map_err(|_| err("bad number"))).collect::<Result<_, _>>()?;
            if v.len() < k { Err(err("too few components")) } else { Ok(v) }
        };
        match it.next() {
            Some("v") => {
                let c = nums(it, 3)?;
                for i in 0..3 {
                    lo[i] = lo[i].min(c[i]);
                    hi[i] = hi[i].max(c[i]);
                }
                pos.push([c[0], c[1], c[2]]);
            }
            Some("vt") => {
                let c = nums(it, 2)?;
                uvs.push([c[0], c[1]]);
            }
            Some("o") | Some("g") => {
                let name = it.collect::<Vec<_>>().join(" ");
                nodes.push(Node::new(Kind::Object, Some(0), if name.is_empty() { "object" } else { &name }, false));
                bounds.push(EMPTY);
                current = Some(nodes.len() - 1);
            }
            Some("gpr_node") => {
                let kind = match it.next() {
                    Some("collection") => Kind::Collection,
                    _ => Kind::Object,
                };
                let parent: usize = it.next().and_then(|s| s.parse().ok()).filter(|&p| p < nodes.len()).ok_or_else(|| err("bad gpr_node parent"))?;
                let hidden = it.next() == Some("1");
                let name = it.collect::<Vec<_>>().join(" ");
                nodes.push(Node::new(kind, Some(parent), &name, hidden));
                bounds.push(EMPTY);
            }
            Some("gpr_use") => {
                let i: usize = it.next().and_then(|s| s.parse().ok()).filter(|&i| i < nodes.len()).ok_or_else(|| err("bad gpr_use"))?;
                current = Some(i);
            }
            Some("usemtl") => {
                // `opaque` / `blend` choose the draw pass (see the exporter); any other name is a material
                let name = it.collect::<Vec<_>>().join(" ");
                in_blend = name == "blend";
                if name != "blend" && name != "opaque" && !name.is_empty() {
                    material = Some(name);
                }
            }
            Some("gpr_material") => {
                let name = it.collect::<Vec<_>>().join(" ");
                material = (!name.is_empty()).then_some(name);
            }
            Some("mtlrect") => {
                let c = nums(it, 4)?;
                rect = [c[0], c[1], c[2], c[3]];
            }
            Some("f") => {
                let mut face: Face = Vec::new();
                for tok in it {
                    // v, v/vt, v//vn, v/vt/vn; 1-based, negatives count from the end
                    let mut p = tok.split('/');
                    let idx = |s: Option<&str>, len: usize| -> Result<Option<usize>, String> {
                        match s {
                            None | Some("") => Ok(None),
                            Some(s) => {
                                let i: i64 = s.parse().map_err(|_| err("bad index"))?;
                                let i = if i < 0 { len as i64 + i } else { i - 1 };
                                if i < 0 || i as usize >= len { Err(err("index out of range")) } else { Ok(Some(i as usize)) }
                            }
                        }
                    };
                    let v = idx(p.next(), pos.len())?.ok_or_else(|| err("missing position"))?;
                    let t = idx(p.next(), uvs.len())?;
                    face.push((pos[v], t.map_or([0.0, 0.0], |i| uvs[i])));
                }
                if face.len() < 3 {
                    return Err(err("face needs 3+ vertices"));
                }
                let node = *current.get_or_insert_with(|| {
                    nodes.push(Node::new(Kind::Object, Some(0), "mesh", false));
                    bounds.push(EMPTY);
                    nodes.len() - 1
                });
                for (p, _) in &face {
                    for k in 0..3 {
                        bounds[node].0[k] = bounds[node].0[k].min(p[k]);
                        bounds[node].1[k] = bounds[node].1[k].max(p[k]);
                    }
                }
                match face.len() {
                    3 => nodes[node].tris += 1,
                    4 => nodes[node].quads += 1,
                    k => nodes[node].tris += k - 2,
                }
                // faces of a named material belong to the material's own node (a child of the object)
                let vnode = match material.clone().filter(|_| nodes[node].kind == Kind::Object) {
                    Some(m) => {
                        let key = (node, m.clone());
                        match mat_nodes.get(&key) {
                            Some(&id) => id,
                            None => {
                                let name = format!("{}::{}", nodes[node].name, m);
                                nodes.push(Node::new(Kind::Material, Some(node), &name, false));
                                bounds.push(EMPTY);
                                mat_nodes.insert(key, nodes.len() - 1);
                                nodes.len() - 1
                            }
                        }
                    }
                    None => node,
                };
                for (p, _) in &face {
                    for k in 0..3 {
                        bounds[vnode].0[k] = bounds[vnode].0[k].min(p[k]);
                        bounds[vnode].1[k] = bounds[vnode].1[k].max(p[k]);
                    }
                }
                push_face(&mut data, if in_blend { &mut blend } else { &mut opaque }, &mut wire, &face, rect, vnode);
            }
            _ => {}
        }
    }
    if opaque.is_empty() && blend.is_empty() {
        return Err("no faces in file".into());
    }
    // Children always come after their parents, so folding in reverse order propagates upward.
    for i in (1..nodes.len()).rev() {
        if let Some(p) = nodes[i].parent {
            let (clo, chi) = bounds[i];
            for k in 0..3 {
                bounds[p].0[k] = bounds[p].0[k].min(clo[k]);
                bounds[p].1[k] = bounds[p].1[k].max(chi[k]);
            }
            let (t, q) = (nodes[i].tris, nodes[i].quads);
            nodes[p].tris += t;
            nodes[p].quads += q;
        }
    }
    for (n, (lo, hi)) in nodes.iter_mut().zip(&bounds) {
        if lo[0] <= hi[0] {
            n.center = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
            n.radius = (math::dot(math::sub(*hi, *lo), math::sub(*hi, *lo)).sqrt() * 0.5).max(1e-6);
        }
    }
    // a material node pivots where its object does, so it follows the object's transform exactly
    for i in 0..nodes.len() {
        if nodes[i].kind == Kind::Material {
            let p = nodes[i].parent.unwrap_or(0);
            (nodes[i].center, nodes[i].radius) = (nodes[p].center, nodes[p].radius);
        }
    }
    let opaque_count = opaque.len();
    opaque.extend(blend);
    let center = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
    let radius = math::dot(math::sub(hi, lo), math::sub(hi, lo)).sqrt() * 0.5;
    Ok(Mesh { nodes, data, indices: opaque, opaque_count, wire, center, radius: radius.max(1e-6) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quads_stay_quads_and_wire_has_four_edges() {
        let m = parse("v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n").unwrap();
        assert_eq!(m.nodes[0].quads, 1);
        assert_eq!(m.indices.len(), 6); // two triangles
        assert_eq!(m.wire.len(), 8); // four outline edges, no diagonal
        assert_eq!(m.data.len(), 4 * FLOATS);
    }

    #[test]
    fn hierarchy_directives() {
        let m = parse("gpr_node collection 0 0 Props\ngpr_node object 1 1 Crate Big\nv 0 0 0\nv 1 0 0\nv 0 1 0\ngpr_use 2\nf 1 2 3\n").unwrap();
        assert_eq!(m.nodes[1].name, "Props");
        assert_eq!(m.nodes[2].name, "Crate Big");
        assert!(m.nodes[2].hidden);
        assert_eq!(m.nodes[1].tris, 1); // propagated to the collection
        assert!(m.nodes[2].radius > 0.0);
    }

    #[test]
    fn materials_become_child_nodes_of_their_object() {
        let m = parse("gpr_node object 0 0 Crate\nv 0 0 0\nv 1 0 0\nv 0 1 0\nv 1 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvt 1 1\ngpr_use 1\ngpr_material Wood\nf 1/1 2/2 3/3\ngpr_material Metal\nf 2/2 4/4 3/3\ngpr_material Wood\nf 1/1 2/2 4/4\n").unwrap();
        let kinds: Vec<_> = m.nodes.iter().map(|n| (n.kind, n.name.as_str())).collect();
        assert_eq!(kinds.len(), 4, "{kinds:?}");
        assert_eq!(m.nodes[2].name, "Crate::Wood");
        assert_eq!((m.nodes[2].kind, m.nodes[2].parent, m.nodes[2].material_name()), (Kind::Material, Some(1), Some("Wood")));
        assert_eq!(m.nodes[3].material_name(), Some("Metal"));
        assert_eq!(m.nodes[2].center, m.nodes[1].center, "same pivot as the object");
        // vertex node ids point at the material nodes, and map back to the object
        let ids: Vec<usize> = m.data.chunks_exact(FLOATS).map(|v| v[FLOATS - 1] as usize).collect();
        assert!(ids.contains(&2) && ids.contains(&3) && !ids.contains(&1));
        assert!(ids.iter().all(|&i| m.object_of(i) == 1));
        assert_eq!(m.nodes[1].tris, 3, "faces are counted on the object");
        let boxes = m.uv_boxes();
        assert_eq!(boxes[3][..2], [0.0, 0.0]);
    }
}
