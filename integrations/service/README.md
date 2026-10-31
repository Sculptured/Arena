# service integration

Each instance is a Docker Compose project `arena_<instance>`, run with
`docker compose -p arena_<instance> -f <compose_file>` (`COMPOSE_PROJECT_NAME` is never passed on).
Config: `{compose_file, volumes: [name], helper_image?, env?}`; `volumes` are the compose file's volumes
that a save holds (letters, digits, `_`, `-`).

- Save = stop the project, copy each volume to `$ARENA_SAVES/service/<save>/<volume>/`, start it again.
  The save is exact. Volumes move as tar streams through a throwaway container of `helper_image`
  (default `redis:6.2-alpine`, never pulled; any image with `tar` works), so the files on the host belong to
  the current user. Files unchanged since the instance's previous save (same size and mtime) are hard-linked.
- Start from a save = a new project whose volumes are created, filled from the save, and then started.
- `stop` runs `docker compose down -v`: the instance and its volumes are gone, so save first.
- No tools. `inspect` key `volume/<name>` gives `{bytes, files, hash}`; `diff` compares per-volume content
  hashes. For a live instance both read the volume's files on disk (a copy made without stopping it).
- `usage` is 0: saves are under `ARENA_SAVES`, and live volumes are not counted (`stop` removes them).
- Live test: `cargo test -p arena-service -- --ignored live_` (needs Docker and `redis:6.2-alpine`).
