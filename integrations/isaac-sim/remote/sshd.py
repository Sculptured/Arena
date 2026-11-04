"""A minimal SSH server, for running this part on a GPU machine whose container has no sshd and no root (the
Isaac Sim image runs as uid 1234), so that arenad can start it with `ARENA_RUN_ISAAC_SIM="ssh ... -- <command>"`.

Key login only (any user name; the keys in AUTHORIZED_KEYS). Each session runs one command with `bash -c`, its
stdin, stdout and stderr copied to and from the session's, as bytes. When the session closes first, the command gets EOF on
stdin and GRACE_S seconds to exit (the part finishes its recording on EOF), then its process group is killed.

    python sshd.py PORT AUTHORIZED_KEYS HOST_KEY      (HOST_KEY is made on first start)

Needs `asyncssh` (`python -m pip install asyncssh`)."""
import asyncio
import os
import signal
import sys

import asyncssh

GRACE_S = 60


def _kill(proc) -> None:
    if proc.returncode is None:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


async def _wait_or_kill(proc) -> int:
    try:
        return await asyncio.wait_for(proc.wait(), GRACE_S)
    except asyncio.TimeoutError:
        _kill(proc)
        return await proc.wait()


async def _pump(reader, writer) -> None:
    """Copies until EOF, waiting for each chunk to be taken: big outputs arrive whole."""
    while chunk := await reader.read(65536):
        writer.write(chunk)
        await writer.drain()


async def _feed(reader, proc) -> None:
    """The session's stdin to the command's, then EOF."""
    try:
        await _pump(reader, proc.stdin)
    except (ConnectionError, asyncssh.Error):
        pass
    finally:
        proc.stdin.close()


async def session(process) -> None:
    """Runs the session's command until it exits and its output is sent, or until the session closes and the grace
    time is over."""
    pipe = asyncio.subprocess.PIPE
    proc = await asyncio.create_subprocess_exec("bash", "-c", process.command or "true", stdin=pipe, stdout=pipe,
                                                stderr=pipe, start_new_session=True)
    feed = asyncio.ensure_future(_feed(process.stdin, proc))
    out = asyncio.gather(_pump(proc.stdout, process.stdout), _pump(proc.stderr, process.stderr))
    closed = asyncio.ensure_future(process.wait_closed())
    try:
        await asyncio.wait({out, closed}, return_when=asyncio.FIRST_COMPLETED)
        code = await _wait_or_kill(proc)
        if not closed.done():  # is_closing() is already true once the client has sent EOF
            process.exit(code)
    finally:
        for task in (feed, out, closed):
            task.cancel()
        _kill(proc)


async def serve(port: int, authorized_keys: str, host_key: str):
    if not os.path.exists(host_key):
        asyncssh.generate_private_key("ssh-ed25519").write_private_key(host_key)
    return await asyncssh.listen("", port, server_host_keys=[host_key], authorized_client_keys=authorized_keys,
                                 process_factory=session, encoding=None)


async def main(argv) -> None:
    port, keys, host_key = int(argv[0]), argv[1], argv[2]
    await serve(port, keys, host_key)
    print(f"sshd listening on {port}", flush=True)
    await asyncio.Future()


if __name__ == "__main__":
    asyncio.run(main(sys.argv[1:]))
