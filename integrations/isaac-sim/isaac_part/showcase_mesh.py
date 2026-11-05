"""Geometry for the showcase scene as plain arrays (no pxr, no GPU): chamfered boxes, cylinders and stamped panels,
merged into one mesh per material so that a machine of hundreds of parts is a handful of prims."""
import math
from dataclasses import dataclass, field

import numpy as np

_CORNERS = np.array([[sx, sy, sz] for sx in (-1, 1) for sy in (-1, 1) for sz in (-1, 1)], dtype=float)
_FACES = (((0, 1), (0, -1), (0, 0)), ((1, 1), (1, -1), (1, 0)), ((2, 1), (2, -1), (2, 0)))


def box_pieces(size, bevel=0.0) -> list:
    """The faces of a box with chamfered edges as (points Nx3, normals Nx3) polygons, centred on the origin. A face
    has its own normal; a chamfer blends its two neighbours' normals, so an edge catches light as a real one does."""
    half = np.asarray(size, dtype=float) / 2
    b = min(float(bevel), float(half.min()) * 0.9)
    pieces = [_box_face(half, b, axis, sign) for axis in range(3) for sign in (1, -1)]
    if b > 0:
        pieces += [_box_chamfer(half, b, a0, a1, s0, s1) for a0, a1 in ((0, 1), (0, 2), (1, 2))
                   for s0 in (1, -1) for s1 in (1, -1)]
        pieces += [_box_corner(half, b, c) for c in _CORNERS]
    return pieces


def _box_face(half, b, axis, sign):
    u, v = (axis + 1) % 3, (axis + 2) % 3
    points = []
    for su, sv in ((-1, -1), (1, -1), (1, 1), (-1, 1)):
        p = np.zeros(3)
        p[axis], p[u], p[v] = sign * half[axis], su * (half[u] - b), sv * (half[v] - b)
        points.append(p)
    normal = np.zeros(3)
    normal[axis] = sign
    points = points if sign > 0 else points[::-1]
    return np.array(points), np.tile(normal, (4, 1))


def _box_chamfer(half, b, a0, a1, s0, s1):
    """The chamfer along the third axis between the faces a0 (side s0) and a1 (side s1)."""
    w = 3 - a0 - a1
    points, normals = [], []
    for sw, on0 in ((-1, True), (1, True), (1, False), (-1, False)):
        p, n = np.zeros(3), np.zeros(3)
        p[w] = sw * (half[w] - b)
        p[a0] = s0 * (half[a0] if on0 else half[a0] - b)
        p[a1] = s1 * (half[a1] - b if on0 else half[a1])
        n[a0 if on0 else a1] = s0 if on0 else s1
        points.append(p)
        normals.append(n)
    return _outward(np.array(points), np.array(normals), np.array([s0 if i == a0 else s1 if i == a1 else 0
                                                                   for i in range(3)], dtype=float))


def _box_corner(half, b, c):
    points, normals = [], []
    for axis in range(3):
        p = c * (half - b)
        p[axis] = c[axis] * half[axis]
        n = np.zeros(3)
        n[axis] = c[axis]
        points.append(p)
        normals.append(n)
    return _outward(np.array(points), np.array(normals), c)


def _outward(points, normals, direction):
    """The polygon wound so that its front faces `direction`."""
    if np.dot(np.cross(points[1] - points[0], points[2] - points[0]), direction) < 0:
        return points[::-1], normals[::-1]
    return points, normals


def cylinder_pieces(radius, height, axis="Z", segments=24, bevel=0.0) -> list:
    """A capped cylinder along `axis`, centred on the origin, with smooth sides and an optional chamfer at the caps."""
    h = height / 2
    b = min(bevel, radius * 0.5, h * 0.9)
    pieces = []
    for i in range(segments):
        a0, a1 = 2 * math.pi * i / segments, 2 * math.pi * (i + 1) / segments
        c0, s0, c1, s1 = math.cos(a0), math.sin(a0), math.cos(a1), math.sin(a1)
        pieces.append((np.array([[radius * c0, radius * s0, -(h - b)], [radius * c1, radius * s1, -(h - b)],
                                 [radius * c1, radius * s1, h - b], [radius * c0, radius * s0, h - b]]),
                       np.array([[c0, s0, 0], [c1, s1, 0], [c1, s1, 0], [c0, s0, 0]], dtype=float)))
        if b > 0:
            pieces += _cylinder_chamfers(radius, h, b, c0, s0, c1, s1)
    r = radius - b
    ring = [(r * math.cos(2 * math.pi * i / segments), r * math.sin(2 * math.pi * i / segments))
            for i in range(segments)]
    pieces.append((np.array([[x, y, h] for x, y in ring]), np.tile([0.0, 0.0, 1.0], (segments, 1))))
    pieces.append((np.array([[x, y, -h] for x, y in reversed(ring)]), np.tile([0.0, 0.0, -1.0], (segments, 1))))
    return [(_axis_swap(p, axis), _axis_swap(n, axis)) for p, n in pieces]


def _cylinder_chamfers(radius, h, b, c0, s0, c1, s1):
    r = radius - b
    top = (np.array([[radius * c0, radius * s0, h - b], [radius * c1, radius * s1, h - b],
                     [r * c1, r * s1, h], [r * c0, r * s0, h]]),
           np.array([[c0, s0, 0], [c1, s1, 0], [0, 0, 1], [0, 0, 1]], dtype=float))
    bottom = (np.array([[r * c0, r * s0, -h], [r * c1, r * s1, -h],
                        [radius * c1, radius * s1, -(h - b)], [radius * c0, radius * s0, -(h - b)]]),
              np.array([[0, 0, -1], [0, 0, -1], [c1, s1, 0], [c0, s0, 0]], dtype=float))
    return [top, bottom]


def _axis_swap(a, axis):
    """Z-up arrays turned so that local Z points along `axis`, keeping the winding."""
    if axis == "Z":
        return a
    if axis == "X":
        return a[..., [2, 0, 1]]
    return a[..., [1, 2, 0]]


def stamped_panel(width_x, width_y, depth, n=28) -> tuple:
    """A formed panel as a height field: a flat flange, a drawn pocket with a raised bead inside, and two embossed
    ribs. Returns (points, normals, faces) with quads; used for parts on belts and in stillages (raw steel)."""
    xs = np.linspace(-0.5, 0.5, n)
    ys = np.linspace(-0.5, 0.5, n)
    gx, gy = np.meshgrid(xs, ys, indexing="ij")
    z = _panel_height(gx, gy) * depth
    points = np.stack([gx * width_x, gy * width_y, z], axis=-1)
    dzdx, dzdy = np.gradient(z, width_x / (n - 1), width_y / (n - 1))
    normals = np.stack([-dzdx, -dzdy, np.ones_like(z)], axis=-1)
    normals /= np.linalg.norm(normals, axis=-1, keepdims=True)
    faces = [(i * n + j, (i + 1) * n + j, (i + 1) * n + j + 1, i * n + j + 1)
             for i in range(n - 1) for j in range(n - 1)]
    return points.reshape(-1, 3), normals.reshape(-1, 3), np.array(faces, dtype=int)


def _panel_height(gx, gy):
    """The form of the panel, -1 (drawn down) .. 0 (flange), over the unit square."""
    edge = np.minimum(0.5 - np.abs(gx), 0.5 - np.abs(gy))
    pocket = np.clip((edge - 0.07) / 0.09, 0.0, 1.0)
    pocket = pocket * pocket * (3 - 2 * pocket)
    bead = np.exp(-(((np.abs(gx) - 0.2) / 0.05) ** 2)) * np.clip(1 - (gy / 0.33) ** 4, 0.0, 1.0)
    return -pocket + 0.45 * bead * pocket


@dataclass
class Merged:
    """Polygons gathered for one mesh: `add` takes pieces with an offset and optional yaw."""
    points: list = field(default_factory=list)
    normals: list = field(default_factory=list)
    counts: list = field(default_factory=list)
    indices: list = field(default_factory=list)
    uvs: list = field(default_factory=list)
    n: int = 0

    def add(self, pieces, at=(0, 0, 0), yaw_deg=0.0):
        c, s = math.cos(math.radians(yaw_deg)), math.sin(math.radians(yaw_deg))
        rot = np.array([[c, -s, 0], [s, c, 0], [0, 0, 1]])
        at = np.asarray(at, dtype=float)
        for points, normals in pieces:
            world = points @ rot.T + at
            turned = normals @ rot.T
            self.points.append(world)
            self.normals.append(turned)
            self.uvs.append(_box_uv(world, turned))
            k = len(points)
            self.counts.append(k)
            self.indices.extend(range(self.n, self.n + k))
            self.n += k

    def box(self, center, size, bevel=0.02, yaw_deg=0.0):
        self.add(box_pieces(size, bevel), center, yaw_deg)

    def cylinder(self, center, radius, height, axis="Z", segments=24, bevel=0.0):
        self.add(cylinder_pieces(radius, height, axis, segments, bevel), center)

    def arrays(self) -> dict:
        return {"points": np.concatenate(self.points), "normals": np.concatenate(self.normals),
                "counts": np.array(self.counts), "indices": np.array(self.indices), "uvs": np.concatenate(self.uvs)}

    def bounds(self) -> tuple:
        pts = np.concatenate(self.points)
        return pts.min(axis=0), pts.max(axis=0)


def _box_uv(points, normals):
    """World-scale box projection, 1 UV unit a metre, so a texture lies evenly across parts of any size."""
    n = np.abs(normals.mean(axis=0))
    axis = int(np.argmax(n))
    u, v = [(1, 2), (0, 2), (0, 1)][axis]
    return points[:, [u, v]]
