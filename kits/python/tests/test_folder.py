import os

from arena_part import folder


def test_copy_save_links_unchanged(tmp_path):
    src = tmp_path / "live"
    src.mkdir()
    (src / "a.txt").write_text("hello")
    (src / "sub").mkdir()
    (src / "sub" / "b.txt").write_text("world")
    s1, s2 = tmp_path / "s1", tmp_path / "s2"
    assert folder.copy_save(src, s1, None) == 10
    assert folder.copy_save(src, s2, s1) == 0
    assert os.stat(s1 / "a.txt").st_ino == os.stat(s2 / "a.txt").st_ino
    assert os.stat(s1 / "sub" / "b.txt").st_ino == os.stat(s2 / "sub" / "b.txt").st_ino
    # A changed file is copied afresh.
    (src / "a.txt").write_text("hello, again")
    s3 = tmp_path / "s3"
    assert folder.copy_save(src, s3, s2) == 12
    assert os.stat(s3 / "a.txt").st_ino != os.stat(s2 / "a.txt").st_ino
    assert (s2 / "a.txt").read_text() == "hello"


def test_restore_never_links(tmp_path):
    save = tmp_path / "s1"
    save.mkdir()
    (save / "a.txt").write_text("hello")
    live = tmp_path / "live"
    folder.restore(save, live)
    assert (live / "a.txt").read_text() == "hello"
    assert os.stat(live / "a.txt").st_ino != os.stat(save / "a.txt").st_ino
    (live / "a.txt").write_text("changed")
    assert (save / "a.txt").read_text() == "hello"
    # Restoring over an existing live folder replaces its files without touching the save.
    folder.restore(save, live)
    assert (live / "a.txt").read_text() == "hello"
