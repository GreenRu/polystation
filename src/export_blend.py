# Run by gpurender through `blender --background <file.blend> --python export_blend.py -- out.obj out.bmp`
#
# Writes every visible mesh (world space, Y-up, quads kept) as OBJ, and packs the simple parts of each
# material - base color, base color texture, alpha (value or texture) - into ONE RGBA atlas (32-bit BMP).
#
# OBJ extensions understood by gpurender (ignored by other OBJ readers):
#   usemtl opaque|blend      which draw pass the following faces belong to
#   mtlrect x y w h          atlas rectangle (pixels, y from the bottom) used by the following faces;
#                            the shader wraps UVs inside it, so tiling textures still tile
import struct
import sys

import bpy
import numpy as np

out_obj, out_bmp = sys.argv[sys.argv.index("--") + 1:][:2]
ATLAS_W = 2048
MAX_ATLAS_H = 4096


def import_foreign(path):
    """Imports a non-.blend model (fbx, gltf, stl, ...) into the empty startup scene."""
    import os
    for ob in list(bpy.data.objects):
        bpy.data.objects.remove(ob, do_unlink=True)
    ext = os.path.splitext(path)[1].lower()
    ops = bpy.ops
    table = {
        ".obj": ["wm.obj_import", "import_scene.obj"],
        ".fbx": ["import_scene.fbx"],
        ".gltf": ["import_scene.gltf"], ".glb": ["import_scene.gltf"],
        ".stl": ["wm.stl_import", "import_mesh.stl"],
        ".ply": ["wm.ply_import", "import_mesh.ply"],
        ".dae": ["wm.collada_import"],
        ".abc": ["wm.alembic_import"],
        ".usd": ["wm.usd_import"], ".usda": ["wm.usd_import"], ".usdc": ["wm.usd_import"], ".usdz": ["wm.usd_import"],
        ".x3d": ["import_scene.x3d"], ".wrl": ["import_scene.x3d"],
        ".3ds": ["import_scene.autodesk_3ds", "import_scene.max3ds"],
        ".svg": ["import_curve.svg"],
    }
    last = None
    for name in table.get(ext, []):
        grp, fn = name.split(".")
        op = getattr(getattr(ops, grp, None), fn, None)
        if op is None:
            continue
        try:
            if op(filepath=path) == {"FINISHED"}:
                print("GPURENDER: imported", path, "with", name)
                return
        except Exception as e:  # try the next importer
            last = e
    raise RuntimeError(f"no Blender importer could read {path}: {last}")


import os as _os
if _os.environ.get("GPR_IMPORT"):
    import_foreign(_os.environ["GPR_IMPORT"])
deps = bpy.context.evaluated_depsgraph_get()


def log(*a):
    print("GPURENDER:", *a)


def srgb(c):
    c = np.clip(c, 0.0, 1.0)
    return np.where(c <= 0.0031308, c * 12.92, 1.055 * np.power(c, 1 / 2.4) - 0.055)


# ---------------------------------------------------------------- material analysis
def principled(mat):
    if not (mat and mat.use_nodes and mat.node_tree):
        return None
    nodes = mat.node_tree.nodes
    for n in nodes:
        if n.type == "OUTPUT_MATERIAL" and n.is_active_output:
            s = n.inputs["Surface"]
            if s.is_linked and s.links[0].from_node.type == "BSDF_PRINCIPLED":
                return s.links[0].from_node
    for n in nodes:
        if n.type == "BSDF_PRINCIPLED":
            return n
    return None


def find_image(sock, depth=4):
    """Image Texture node feeding `sock`, looking through simple colour nodes. -> (node, from_socket)"""
    if not sock.is_linked or depth == 0:
        return None
    link = sock.links[0]
    n = link.from_node
    if n.type == "TEX_IMAGE" and n.image:
        return n, link.from_socket
    for i in n.inputs:
        if i.is_linked and (i.type == "RGBA" or n.type == "REROUTE"):
            r = find_image(i, depth - 1)
            if r:
                return r
    return None


def spec_for(mat):
    """Hashable description of the simple material parts we support."""
    color, alpha, img, aimg, amode = [0.8, 0.8, 0.8], 1.0, None, None, None
    if mat:
        color, alpha = list(mat.diffuse_color[:3]), mat.diffuse_color[3]
    p = principled(mat)
    if p:
        bc = p.inputs["Base Color"]
        found = find_image(bc)
        if found:
            img = found[0].image
        elif not bc.is_linked:
            color = list(bc.default_value[:3])
        al = p.inputs["Alpha"]
        f = find_image(al)
        if f:
            aimg, amode = f[0].image, ("alpha" if f[1].name == "Alpha" else "color")
        elif not al.is_linked:
            alpha = al.default_value
        else:
            alpha = 1.0
    return (tuple(round(float(c), 4) for c in color), round(float(alpha), 4),
            img.name if img else None, aimg.name if aimg else None, amode)


# ---------------------------------------------------------------- pixels
_pix = {}


def get_pixels(name):
    """(h, w, 4) float array, rows bottom-up, colour in display (sRGB) space; None if unavailable."""
    if name in _pix:
        return _pix[name]
    img = bpy.data.images[name]
    if not img.has_data:
        try:
            img.reload()  # images are not loaded in --background until touched
        except Exception as e:
            log("could not load image", name, e)
    out = None
    w, h = img.size
    if img.has_data and w > 0 and h > 0:
        a = np.empty(w * h * 4, np.float32)
        img.pixels.foreach_get(a)
        a = a.reshape(h, w, 4)
        if img.is_float and img.colorspace_settings.name != "Non-Color":
            a[..., :3] = srgb(a[..., :3])
        out = a
    _pix[name] = out
    return out


def resize(a, w, h):
    sh, sw = a.shape[:2]
    ys = np.arange(h) * sh // h
    xs = np.arange(w) * sw // w
    return a[ys][:, xs]


def make_tile(spec, cap):
    color, alpha, iname, aname, amode = spec
    base = get_pixels(iname) if iname else None
    amap = get_pixels(aname) if aname else None
    src = base if base is not None else amap
    if src is None:
        tw = th = 2
    else:
        h, w = src.shape[:2]
        s = min(1.0, cap / max(w, h))
        tw, th = max(1, int(w * s)), max(1, int(h * s))
    tile = np.ones((th, tw, 4), np.float32)
    if base is not None:
        tile[...] = resize(base, tw, th)
    else:
        tile[..., :3] = srgb(np.array(color, np.float32))
    if amode is None:
        tile[..., 3] = alpha
    elif amap is None:
        tile[..., 3] = 1.0
    else:
        m = resize(amap, tw, th)
        tile[..., 3] = m[..., 3] if amode == "alpha" else m[..., 0]
    return tile


def classify(tile):
    """'opaque' (also alpha-cutout) or 'blend' (smooth translucency)."""
    a = tile[..., 3]
    if a.min() >= 0.99:
        return "opaque"
    mid = float(((a > 0.05) & (a < 0.95)).mean())
    return "opaque" if mid < 0.05 else "blend"


# ---------------------------------------------------------------- gather geometry
meshes = []  # (evaluated mesh copy data) kept as plain python so we can pack after seeing every material
spec_ids, specs = {}, []


def spec_id(mat):
    s = spec_for(mat)
    if s not in spec_ids:
        spec_ids[s] = len(specs)
        specs.append(s)
    return spec_ids[s]


verts, uvs, faces = [], [], []  # faces: (spec_id, node_index, [(v, vt or None), ...])
nodes = [("collection", 0, 0, "Scene")]  # (kind, parent_index, hidden, name); index 0 = scene root
seen_objects = set()


def clean(name):
    return " ".join(name.replace("#", "_").split()) or "unnamed"


def add_object(ob, parent):
    if ob.type != "MESH" or ob.name in seen_objects:
        return
    seen_objects.add(ob.name)
    try:
        eob = ob.evaluated_get(deps)
        me = eob.to_mesh()
    except Exception as e:  # e.g. objects in excluded collections have no evaluated copy
        log("skipping", ob.name, e)
        return
    node = len(nodes)
    nodes.append(("object", parent, 0 if ob.visible_get() else 1, clean(ob.name)))
    uv = me.uv_layers.active
    mw = eob.matrix_world
    voff, toff = len(verts), len(uvs)
    for v in me.vertices:
        p = mw @ v.co
        verts.append((p.x, p.z, -p.y))  # Blender Z-up -> Y-up
    if uv:
        uvs.extend((d.uv.x, d.uv.y) for d in uv.data)
    slot_ids = {}
    for poly in me.polygons:
        mi = poly.material_index
        if mi not in slot_ids:
            mat = me.materials[mi] if mi < len(me.materials) else None
            slot_ids[mi] = spec_id(mat)
        loops = []
        for li in poly.loop_indices:
            loops.append((me.loops[li].vertex_index + 1 + voff, li + 1 + toff if uv else None))
        faces.append((slot_ids[mi], node, loops))
    eob.to_mesh_clear()


def walk(col, parent):
    for ob in col.objects:
        add_object(ob, parent)
    for child in col.children:
        idx = len(nodes)
        nodes.append(("collection", parent, 0, clean(child.name)))
        walk(child, idx)


walk(bpy.context.scene.collection, 0)
log(f"{sum(1 for n in nodes if n[0] == 'object')} object(s)")

# ---------------------------------------------------------------- build + pack atlas
cap = 1024
while True:
    tiles = [make_tile(s, cap) for s in specs]
    order = sorted(range(len(tiles)), key=lambda i: -tiles[i].shape[0])
    pos, x, y, rowh = {}, 0, 0, 0
    for i in order:
        h, w = tiles[i].shape[:2]
        if x + w > ATLAS_W:
            x, y, rowh = 0, y + rowh, 0
        pos[i] = (x, y)
        x += w
        rowh = max(rowh, h)
    atlas_h = max(1, y + rowh)
    if atlas_h <= MAX_ATLAS_H or cap <= 16:
        break
    cap //= 2

atlas = np.zeros((atlas_h, ATLAS_W, 4), np.float32)
for i, t in enumerate(tiles):
    x0, y0 = pos[i]
    atlas[y0:y0 + t.shape[0], x0:x0 + t.shape[1]] = t
modes = [classify(t) for t in tiles]
log(f"{len(specs)} material(s), atlas {ATLAS_W}x{atlas_h}, tile cap {cap}")

px = (np.clip(atlas, 0, 1) * 255 + 0.5).astype(np.uint8)
bgra = px[..., [2, 1, 0, 3]].tobytes()  # rows already bottom-up, as BMP stores them
with open(out_bmp, "wb") as f:
    f.write(b"BM" + struct.pack("<IHHI", 54 + len(bgra), 0, 0, 54))
    f.write(struct.pack("<IiiHHIIiiII", 40, ATLAS_W, atlas_h, 1, 32, 0, len(bgra), 2835, 2835, 0, 0))
    f.write(bgra)
log("texture atlas written")

# ---------------------------------------------------------------- write OBJ
lines = [f"v {x:.6f} {y:.6f} {z:.6f}" for x, y, z in verts]
lines += [f"vt {u:.6f} {v:.6f}" for u, v in uvs]
lines += [f"gpr_node {kind} {parent} {hidden} {name}" for kind, parent, hidden, name in nodes[1:]]
by_spec = {sid: [] for sid in range(len(specs))}
for fsid, node, loops in faces:
    by_spec[fsid].append((node, loops))
for lst in by_spec.values():
    lst.sort(key=lambda fl: fl[0])
for group in ("opaque", "blend"):
    lines.append(f"usemtl {group}")
    for sid in range(len(specs)):
        if modes[sid] != group:
            continue
        x0, y0 = pos[sid]
        t = tiles[sid]
        lines.append(f"mtlrect {x0} {y0} {t.shape[1]} {t.shape[0]}")
        last = -1
        for node, loops in by_spec[sid]:
            if node != last:
                lines.append(f"gpr_use {node}")
                last = node
            lines.append("f " + " ".join(f"{v}/{t_}" if t_ else f"{v}" for v, t_ in loops))
with open(out_obj, "w") as f:
    f.write("\n".join(lines) + "\n")
log("done")
