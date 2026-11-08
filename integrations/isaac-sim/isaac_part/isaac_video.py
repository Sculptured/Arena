"""A recording on the `isaac` backend, driven on the arena clock instead of run.py's wall clock: a Replicator render
product on the main camera, piped to ffmpeg (the factory's `record.py`), headless RTX (RaytracedLighting). What is on
screen is the factory's own visuals (`visuals.py`) or, with `scene: showcase`, the showcase's (`showcase_visuals.py`)."""
import atexit
import importlib
import time

from arena_part import sources

from . import isaac_backend, recording
from .log import log


def _factory_view(cfg, stage, labels):
    """The factory's tower lights, labels, crate, truck and camera director."""
    visuals = sources.import_read_only(cfg.isaac_path, "visuals", "isaac_path")
    view = visuals.Visuals(stage, cfg.camera, label_dir=labels, log=log)

    def show(factory, events, now, _extras=None):
        view.apply(factory.visual_state(), events, now)
    return show, None


def _showcase_view(cfg, stage, labels):
    from .showcase_visuals import ShowcaseVisuals
    view = ShowcaseVisuals(stage, isaac_backend.dressed(), cfg.camera, labels, log)
    return view.show, view


class Video:
    def __init__(self, cfg, _io, physics_hz, path):
        record = sources.import_read_only(cfg.isaac_path, "record", "isaac_path")
        layout = importlib.import_module("layout")
        stage = importlib.import_module("omni.usd").get_context().get_stage()
        # Textures made for the frames go beside the MP4, never into the factory folder.
        labels = path.with_name(path.stem + "_labels")
        make = _showcase_view if cfg.scene == "showcase" else _factory_view
        self.show, self.view = make(cfg, stage, labels)
        width, height = recording.frame_size(cfg.record_height)
        self.rec = record.Recorder(layout.PRIMS["main_camera"], str(path), width, height, cfg.record_fps)
        self.every = recording.frame_every(physics_hz, cfg.record_fps)
        self.hz, self.seq, self.path, self.frames, self.closed = physics_hz, 0, path, 0, False
        self.visuals_s = self.render_s = 0.0
        atexit.register(self.close)  # the MP4 is only playable once ffmpeg has finished it
        log(f"[isaac-sim] recording {width}x{height}@{cfg.record_fps} scene {cfg.scene} (camera {cfg.camera}) to {path}")

    def _new_events(self, plant) -> list:
        events = plant.factory.events_since(self.seq, 500)
        if events:
            self.seq = events[-1]["seq"]
        return events

    def frame(self, plant) -> None:
        """The visuals for the factory's state and its new events, then one rendered frame, at the plant's time."""
        t0 = time.perf_counter()
        now = plant.steps / self.hz
        self.show(plant.factory, self._new_events(plant), now, plant.act(plant.equipment.visual_state))
        t1 = time.perf_counter()
        if self.rec.capture(now):
            self.frames += 1
        self.visuals_s += t1 - t0
        self.render_s += time.perf_counter() - t1

    def stills(self, plant, shots, folder) -> None:
        """One image of each named shot of the showcase, with the plant where it is now. Needs Pillow."""
        from PIL import Image
        folder.mkdir(parents=True, exist_ok=True)
        now = plant.steps / self.hz
        self.show(plant.factory, self._new_events(plant), now, plant.act(plant.equipment.visual_state))
        for name in shots:
            self.view.aim(name)
            for _ in range(recording.STILL_SETTLE_FRAMES):
                self.rec.step()
            data = self.rec.annotator.get_data()
            Image.fromarray(data[:, :, :3]).save(folder / f"{name}.jpg", quality=88)
            log(f"[isaac-sim] still {folder / name}.jpg")

    def stats(self) -> dict:
        """What frames cost so far: the part in Python and the render (which includes the copy to ffmpeg)."""
        n = max(self.frames, 1)
        return {"frames": self.frames, "visuals_ms_per_frame": round(1000 * self.visuals_s / n, 2),
                "render_ms_per_frame": round(1000 * self.render_s / n, 1)}

    def close(self) -> None:
        if self.closed:
            return
        self.closed = True
        out = self.rec.close()
        log(f"[isaac-sim] wrote {out} ({self.frames} frames rendered, {self.rec.sink.written} written, {self.stats()})")
