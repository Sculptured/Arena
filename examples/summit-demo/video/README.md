# Summit video

A narrated walk-through of the summit demo (World → Disrupt → Run → Fail → Rewind → Change →
Compare → Batch), recorded from the real Arena UI on the demo arenad. The agents are scripted,
and every frame says so.

1. Narration: `script.yaml` → Kokoro (`../../factory-showcase/video/narrate_kokoro.py`) writes one
   WAV per scene, plus `timings.json` with each sentence's start.
2. Clips: `record.mjs` drives Chromium at 1600×900, scale 1.2 (1920×1080 frames from Chrome's
   screencast). An in-page overlay (`overlay.js`) draws the cursor, click ripples, and a spotlight
   with an explanation card. Each action starts on the sentence that explains it. `disrupt` starts
   Team v1's live run from the New run dialog, and `rewind` forks it to Team v2 through Rewind here.
   Both need `python3 ../drive.py watch` running. Run ids are kept in `<work>/state.json`.
3. Edit: `edit.yaml` → `../../factory-showcase/video/build.py` (captions, watermark, chapters), then
   loudnorm to -16 LUFS.

```bash
W=/home/alexsky2/sculptured-work-local/video/summit
HF_HUB_OFFLINE=1 <kokoro venv>/python ../../factory-showcase/video/narrate_kokoro.py script.yaml --out $W/narration
NODE_PATH=<node_modules with playwright> node record.mjs            # or: node record.mjs fail rewind
(cd ../../factory-showcase/video && uv run build.py ../../summit-demo/video/edit.yaml)
ffmpeg -i $W/out/summit-demo.mp4 -map 0:v -map 0:a -map_chapters 0 -c:v copy \
  -af loudnorm=I=-16:TP=-1.5:LRA=11 -c:a aac -b:a 192k -movflags +faststart $W/out/summit-demo-final.mp4
```

If you change a scene's narration, keep its sentence count in step with that scene's beats in
`record.mjs`.
