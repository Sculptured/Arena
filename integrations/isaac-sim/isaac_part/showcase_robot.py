"""Kinematics of the press-tending arms, plain numpy: forward kinematics from the joints an asset declares, numeric
inverse kinematics for the few poses a cycle goes through, and joint-space interpolation over a press cycle. The
arms are visuals: nothing here touches PhysX, and a whole cycle costs a few matrix products per frame."""
import math
from dataclasses import dataclass

import numpy as np

AXES = {"X": np.array([1.0, 0.0, 0.0]), "Y": np.array([0.0, 1.0, 0.0]), "Z": np.array([0.0, 0.0, 1.0])}


def quat_matrix(wxyz) -> np.ndarray:
    w, x, y, z = (float(c) for c in wxyz)
    n = math.sqrt(w * w + x * x + y * y + z * z) or 1.0
    w, x, y, z = w / n, x / n, y / n, z / n
    return np.array([[1 - 2 * (y * y + z * z), 2 * (x * y - w * z), 2 * (x * z + w * y)],
                     [2 * (x * y + w * z), 1 - 2 * (x * x + z * z), 2 * (y * z - w * x)],
                     [2 * (x * z - w * y), 2 * (y * z + w * x), 1 - 2 * (x * x + y * y)]])


def pose(pos=(0.0, 0.0, 0.0), rot=None) -> np.ndarray:
    """A 4x4 from a position and a 3x3 rotation (identity if None)."""
    m = np.eye(4)
    m[:3, 3] = pos
    if rot is not None:
        m[:3, :3] = rot
    return m


def axis_rotation(axis, angle) -> np.ndarray:
    a = np.asarray(axis, dtype=float)
    k = np.array([[0, -a[2], a[1]], [a[2], 0, -a[0]], [-a[1], a[0], 0]])
    return np.eye(3) + math.sin(angle) * k + (1 - math.cos(angle)) * k @ k


def yaw_rotation(angle) -> np.ndarray:
    return axis_rotation(AXES["Z"], angle)


@dataclass(frozen=True)
class Joint:
    """A revolute joint as USD physics declares it: child = parent * J0 * R(axis, q) * inverse(J1)."""
    parent: str
    child: str
    j0: np.ndarray
    j1_inv: np.ndarray
    axis: np.ndarray
    lower: float  # radians
    upper: float


def joint_from_usd(parent, child, pos0, rot0, pos1, rot1, axis, lower_deg, upper_deg) -> Joint:
    return Joint(parent, child, pose(pos0, quat_matrix(rot0)), np.linalg.inv(pose(pos1, quat_matrix(rot1))),
                 AXES[axis], math.radians(lower_deg), math.radians(upper_deg))


class Chain:
    """A serial arm: `root_links` keeps fixed poses of the links above the first joint (relative to the arm's root),
    `joints` go from the root out. The tool hangs from the last link: `tool` is its pose in that link's frame."""

    def __init__(self, root_pose, joints, tool=None):
        self.root_pose = root_pose
        self.joints = list(joints)
        self.tool = np.eye(4) if tool is None else tool
        self.lower = np.array([j.lower for j in self.joints])
        self.upper = np.array([j.upper for j in self.joints])

    def links(self, q) -> dict:
        """Every link's pose relative to the arm's root, for joint angles `q` in radians."""
        out = {self.joints[0].parent: self.root_pose}
        for joint, angle in zip(self.joints, q):
            spin = pose(rot=axis_rotation(joint.axis, float(angle)))
            out[joint.child] = out[joint.parent] @ joint.j0 @ spin @ joint.j1_inv
        return out

    def last_link(self, q) -> np.ndarray:
        return self.links(q)[self.joints[-1].child]

    def tool_pose(self, q) -> np.ndarray:
        return self.last_link(q) @ self.tool

    def with_world_aligned_tool(self, offset) -> "Chain":
        """The chain with a tool that is aligned with the arm's root when all joints are at zero, at `offset` from
        the last link in that frame. An arm stretched out along +X then holds a boom along +X, cups down."""
        rest = self.last_link(np.zeros(len(self.joints)))
        return Chain(self.root_pose, self.joints, np.linalg.inv(rest) @ pose(rest[:3, 3] + np.asarray(offset)))


def _error(current, target) -> np.ndarray:
    return np.concatenate([target[:3, 3] - current[:3, 3], _rotation_vector(target[:3, :3] @ current[:3, :3].T)])


def _rotation_vector(rel) -> np.ndarray:
    """The rotation as axis times angle, right up to a half turn (which the plain antisymmetric part loses)."""
    anti = np.array([rel[2, 1] - rel[1, 2], rel[0, 2] - rel[2, 0], rel[1, 0] - rel[0, 1]])
    angle = math.atan2(0.5 * np.linalg.norm(anti), 0.5 * (np.trace(rel) - 1.0))
    if angle < 1e-6:
        return 0.5 * anti
    if angle > math.pi - 1e-3:
        column = int(np.argmax(np.diag(rel)))
        axis = rel[:, column] + np.eye(3)[column]
        return math.pi * axis / np.linalg.norm(axis)
    return anti / np.linalg.norm(anti) * angle


def solve(chain, target, seed, iterations=300, tol=1e-4) -> np.ndarray:
    """Damped least squares from `seed` to the tool pose `target`, within the joint limits. Raises ValueError when it
    can't reach it: a layout the arm can't serve must not build silently."""
    q = np.clip(np.asarray(seed, dtype=float), chain.lower, chain.upper)
    n, eps = len(q), 1e-5
    for _ in range(iterations):
        err = _error(chain.tool_pose(q), target)
        if np.linalg.norm(err) < tol:
            return q
        jac = np.zeros((6, n))
        for i in range(n):
            bumped = q.copy()
            bumped[i] += eps
            jac[:, i] = (err - _error(chain.tool_pose(bumped), target)) / eps
        step = jac.T @ np.linalg.solve(jac @ jac.T + 1e-3 * np.eye(6), err)
        q = np.clip(q + np.clip(step, -0.3, 0.3), chain.lower, chain.upper)
    raise ValueError(f"no joint angles reach {target[:3, 3].round(3).tolist()}, error "
                     f"{np.linalg.norm(_error(chain.tool_pose(q), target)):.3f}")


def smooth(t: float) -> float:
    t = min(1.0, max(0.0, t))
    return t * t * (3 - 2 * t)


class CycleMotion:
    """Joint angles as a function of the press cycle: keys are (phase in 0..1, q), repeating each cycle, with eased
    moves between them, so the arm always moves in step with the crank and stops when the crank stops."""

    def __init__(self, keys):
        self.phases = np.array([p for p, _ in keys], dtype=float)
        self.angles = [np.asarray(q, dtype=float) for _, q in keys]
        if np.any(np.diff(self.phases) <= 0) or self.phases[0] < 0 or self.phases[-1] >= 1:
            raise ValueError("cycle keys need increasing phases in 0..1")

    def at(self, phase: float) -> np.ndarray:
        p = phase % 1.0
        i = int(np.searchsorted(self.phases, p, side="right")) - 1
        n = len(self.phases)
        p0, q0 = self.phases[i % n] - (1.0 if i < 0 else 0.0), self.angles[i % n]
        p1, q1 = self.phases[(i + 1) % n] + (1.0 if i + 1 >= n else 0.0), self.angles[(i + 1) % n]
        return q0 + (q1 - q0) * smooth((p - p0) / (p1 - p0))
