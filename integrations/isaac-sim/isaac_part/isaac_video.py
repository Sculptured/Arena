"""A recording on the `isaac` backend: the factory's own visuals (tower lights, labels, crate, truck and the camera
director, `visuals.py`) and recorder (`record.py`: a Replicator render product on the main camera, piped to ffmpeg),
driven on the arena clock instead of run.py's wall clock. Headless RTX (RaytracedLighting), as run.py records."""
import atexit
import importlib

from arena_part import sources

from . import recording
from .log import log

WIDTH, HEIGHT = 1280, 720


class Video:
    def __init__(self, cfg, _io, physics_hz, path):
        visuals = sources.import_read_only(cfg.isaac_path, "visuals", "isaac_path")
        record = sources.import_read_only(cfg.isaac_path, "record", "isaac_path")
        layout = importlib.import_module("layout")
        stage = importlib.import_module("omni.usd").get_context().get_stage()
        # Label textures go beside the MP4, never into the factory folder.
        labels = path.with_name(path.stem + "_labels")
        self.view = visuals.Visuals(stage, cfg.camera, label_dir=labels, log=log)
        self.rec = record.Recorder(layout.PRIMS["main_camera"], str(path), WIDTH, HEIGHT, cfg.record_fps)
        self.every = recording.frame_every(physics_hz, cfg.record_fps)
        self.hz, self.seq, self.path, self.frames, self.closed = physics_hz, 0, path, 0, False
        atexit.register(self.close)  # the MP4 is only playable once ffmpeg has finished it
        log(f"[isaac-sim] recording {WIDTH}x{HEIGHT}@{cfg.record_fps} (camera {cfg.camera}) to {path}")

    def frame(self, plant) -> None:
        """The visuals for the factory's state and its new events, then one rendered frame, at the plant's time."""
        events = plant.factory.events_since(self.seq, 500)
        if events:
            self.seq = events[-1]["seq"]
        now = plant.steps / self.hz
        self.view.apply(plant.factory.visual_state(), events, now)
        if self.rec.capture(now):
            self.frames += 1

    def close(self) -> None:
        if self.closed:
            return
        self.closed = True
        out = self.rec.close()
        log(f"[isaac-sim] wrote {out} ({self.frames} frames rendered, {self.rec.sink.written} written)")
