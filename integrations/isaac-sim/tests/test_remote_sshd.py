"""remote/sshd.py, the SSH server a GPU container without sshd runs: key login, one command per session over the
session's stdio. Run with `--with asyncssh`; skipped without it or without OpenSSH's client."""
import shutil
import socket
import subprocess
import sys
from pathlib import Path

import pytest

pytest.importorskip("asyncssh")
SSHD = Path(__file__).resolve().parents[1] / "remote" / "sshd.py"
pytestmark = pytest.mark.skipif(not (shutil.which("ssh") and shutil.which("ssh-keygen")), reason="no OpenSSH")


def keypair(path: Path) -> Path:
    subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(path)], check=True)
    return path


@pytest.fixture
def server(tmp_path):
    key = keypair(tmp_path / "client")
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    proc = subprocess.Popen([sys.executable, str(SSHD), str(port), str(key) + ".pub", str(tmp_path / "host")],
                            stdout=subprocess.PIPE, text=True)
    assert "listening" in proc.stdout.readline()
    yield port, key
    proc.kill()
    proc.wait()


def ssh(port, key, command, stdin=""):
    return subprocess.run(["ssh", "-p", str(port), "-i", str(key), "-o", "StrictHostKeyChecking=no", "-o",
                           "UserKnownHostsFile=/dev/null", "-o", "BatchMode=yes", "-o", "LogLevel=ERROR",
                           "arena@127.0.0.1", "--", command], input=stdin, capture_output=True, text=True,
                          timeout=30)


def test_a_session_runs_its_command_over_its_stdio(server):
    port, key = server
    out = ssh(port, key, "read line; echo got:$line; echo to-stderr >&2; exit 3", "hello\n")
    assert (out.stdout, out.stderr.strip(), out.returncode) == ("got:hello\n", "to-stderr", 3)


def test_another_key_is_refused(server, tmp_path):
    port, _ = server
    out = ssh(port, keypair(tmp_path / "stranger"), "echo in")
    assert out.returncode == 255 and "in" not in out.stdout


def test_a_large_output_arrives_whole(server):
    """The recording comes back this way: 21 MB on the 2026-11-04 run."""
    port, key = server
    out = subprocess.run(["ssh", "-p", str(port), "-i", str(key), "-o", "StrictHostKeyChecking=no", "-o",
                          "UserKnownHostsFile=/dev/null", "-o", "BatchMode=yes", "-o", "LogLevel=ERROR",
                          "arena@127.0.0.1", "--", "head -c 25000000 /dev/urandom | tee >(sha256sum >&2)"],
                         capture_output=True, timeout=60)
    import hashlib
    assert len(out.stdout) == 25_000_000
    assert hashlib.sha256(out.stdout).hexdigest() == out.stderr.split()[0].decode()
