"""python -m isaac_part [--backend isaac|standin] [--mode plant|mirror]"""
import argparse
import os
import sys

from arena_part import serve

from .backends import BACKENDS


def protocol_stdout():
    """The protocol's own copy of stdout. Anything else written to stdout (the factory's log, Isaac Sim, C
    libraries) goes to stderr, the log, so it can't break the protocol's lines."""
    proto = os.fdopen(os.dup(1), "w")
    os.dup2(2, 1)
    sys.stdout = sys.stderr
    return proto


def main(argv=None):
    ap = argparse.ArgumentParser(prog="isaac_part", description=__doc__)
    ap.add_argument("--backend", choices=sorted(BACKENDS), default="isaac")
    ap.add_argument("--mode", choices=["plant", "mirror"], default="plant")
    args = ap.parse_args(argv)
    if args.mode == "mirror":
        from .mirror import Mirror
        part = Mirror()
    else:
        from .part import IsaacSim
        part = IsaacSim(BACKENDS[args.backend])
    serve(part, stdout=protocol_stdout())


if __name__ == "__main__":
    main()
