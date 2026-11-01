"""The helpers integrations share: read-only source imports and hashes, tool params, save folders."""
import sys
from datetime import datetime

import pytest

from arena_part import PartError, params, saves, sources


def test_import_read_only_writes_no_bytecode(tmp_path):
    pkg = tmp_path / "ro_src" / "kitpkg_ro"
    pkg.mkdir(parents=True)
    (pkg / "__init__.py").write_text("")
    (pkg / "mod_ro.py").write_text("VALUE = 42\n")
    mod = sources.import_read_only(tmp_path / "ro_src", "kitpkg_ro.mod_ro", "src_path")
    assert mod.VALUE == 42
    assert not list((tmp_path / "ro_src").rglob("__pycache__"))
    assert sys.dont_write_bytecode


def test_import_read_only_names_a_missing_folder(tmp_path):
    gone = tmp_path / "nowhere"
    with pytest.raises(PartError) as e:
        sources.import_read_only(gone, "kitpkg_x.market", "hub_path")
    assert e.value.code == -32602
    assert e.value.message == f"hub_path {gone} has no kitpkg_x/market.py"


def test_import_read_only_reports_a_broken_module(tmp_path):
    (tmp_path / "kit_broken_mod.py").write_text("import no_such_module_anywhere\n")
    with pytest.raises(PartError) as e:
        sources.import_read_only(tmp_path, "kit_broken_mod", "src_path")
    assert e.value.code == -32602 and "kit_broken_mod" in e.value.message and str(tmp_path) in e.value.message


def test_hashes_and_check_unchanged(tmp_path):
    a, b = tmp_path / "a.py", tmp_path / "b.json"
    a.write_text("x = 1\n")
    b.write_text("{}")
    declared = sources.hashes({"a.py": a, "b.json": b})
    assert all(len(h) == 64 for h in declared.values())
    assert sources.file_hash(tmp_path / "gone") == f"missing: {tmp_path / 'gone'}"
    sources.check_unchanged(declared, {"a.py": a, "b.json": b})
    a.write_text("x = 2\n")
    with pytest.raises(PartError) as e:
        sources.check_unchanged(declared, {"a.py": a, "b.json": b})
    assert e.value.code == -32003
    assert e.value.message == f"a.py at {a} differs from the one the fingerprint declares"
    with pytest.raises(PartError) as e:
        sources.check_unchanged(declared, {"a.py": tmp_path / "gone.py"}, "check src_path")
    assert e.value.code == -32602
    assert e.value.message == f"a.py not found at {tmp_path / 'gone.py'} (check src_path)"


def test_params_schema_and_normalize():
    ps = (params.Param("machine", "string"), params.Param("minutes", "integer", 30),
          params.Param("before_trip", "boolean", True), params.Param("rate", "number", None))
    assert params.input_schema(ps) == {
        "type": "object", "required": ["machine"],
        "properties": {"machine": {"type": "string"}, "minutes": {"type": "integer"},
                       "before_trip": {"type": "boolean"}, "rate": {"type": "number"}}}
    assert params.normalize("t", ps, {"machine": "PR-A1", "rate": 1.5}) == {
        "machine": "PR-A1", "minutes": 30, "before_trip": True, "rate": 1.5}
    assert params.normalize("t", ps, {"machine": "m", "rate": None})["rate"] is None
    for args, text in [({}, "t: missing machine"), ({"machine": "m", "minutes": True}, "minutes"),
                       ({"machine": "m", "before_trip": 1}, "before_trip"), ([], "args must be an object")]:
        with pytest.raises(params.Refusal, match=text):
            params.normalize("t", ps, args)
    assert params.refused("no") == {"error": "no", "reads": [], "writes": []}


def test_config_datetime():
    assert params.config_datetime({}, "plant_start", "2026-03-10T06:00:00") == datetime(2026, 3, 10, 6)
    assert params.config_datetime({"plant_start": "2026-01-02T03:04:05"}, "plant_start", "x").second == 5
    with pytest.raises(PartError, match="plant_start must be an ISO datetime"):
        params.config_datetime({"plant_start": "tomorrow-ish"}, "plant_start", "2026-03-10T06:00:00")


def test_saves_root_drop_and_gc(saves_root):
    assert saves.root("pkg") == saves_root / "pkg"
    for s in ("base_1", "tmp_1", "tmp_2"):
        (saves_root / "pkg" / s).mkdir(parents=True)
        (saves_root / "pkg" / s / "f").write_bytes(b"12345")
    saves.drop("pkg", "tmp_2")
    assert not (saves_root / "pkg" / "tmp_2").exists()
    assert saves.gc("pkg", ["base_*"]) == {"dropped": 1, "bytes": 5}
    assert (saves_root / "pkg" / "base_1").is_dir() and not (saves_root / "pkg" / "tmp_1").exists()
    assert saves.gc("never-saved", []) == {"dropped": 0, "bytes": 0}


def test_saves_gc_skips_folders_that_are_not_saves(saves_root):
    base = saves_root / "pkg"
    for name in ("s_1", ".live", ".tmp", "Not-An-Id", "x" * 64):
        (base / name).mkdir(parents=True)
    (base / "loose_file").write_text("x")
    assert saves.gc("pkg", []) == {"dropped": 1, "bytes": 0}
    left = sorted(p.name for p in base.iterdir())
    assert left == [".live", ".tmp", "Not-An-Id", "loose_file", "x" * 64]
