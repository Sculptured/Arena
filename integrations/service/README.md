# service integration

Puts any Docker Compose project in the world: one project per instance, with the volumes you list. It is the fallback
for a system that has no save code of its own. Restore: exact from a save (stop, copy the volumes, start); approximate:
no. `uses_clock`: false. Verified live: `live_service_roundtrip` ran once against a real Redis 6.2 project in Docker
(save, change, restore, file owners kept, save and stop through a restarted program). It has no `arena-part-check` run:
`check/` is empty, because the part has no tools and needs a compose file in its config.

Each instance is a Docker Compose project `arena_<instance>`, run with
`docker compose -p arena_<instance> -f <compose_file>` (`COMPOSE_PROJECT_NAME` is never passed on).
Config: `{compose_file, volumes: [name], helper_image?, env?}`; `volumes` are the compose file's volumes
that a save holds (letters, digits, `_`, `-`).

- Save = stop the project, write each volume to `$ARENA_SAVES/service/<save>/<volume>.tar`, start it again.
  The archive is exactly what a throwaway container of `helper_image` (default `redis:6.2-alpine`, never
  pulled; any image with `tar` works) streams: owners (numeric), modes and mtimes are kept. No hard links:
  every save holds full archives.
- Start from a save = a new project whose volumes are created, filled from the archives (extracted as root, so
  owners survive), and then started.
- Each running instance's config is kept in `$ARENA_SAVES/service/.live/<instance>.json` (removed by `stop`), so a
  restarted program can still save, inspect and stop it.
- `stop` runs `docker compose down -v`: the instance and its volumes are gone, so save first.
- No tools. `inspect` key `volume/<name>` gives `{bytes, files, hash}`; `diff` compares per-volume hashes of the
  archive's entries (name, owner, mode, mtime, content; folder mtimes ignored). For a live instance both read the
  volume's files on disk (an archive made without stopping it).
- `usage` is 0: saves are under `ARENA_SAVES`, and live volumes are not counted (`stop` removes them).
- Live test: `cargo test -p arena-service -- --ignored live_` (needs Docker and `redis:6.2-alpine`).
