"""The part's log: stderr (stdout carries the protocol)."""
import sys


def log(*args):
    print(*args, file=sys.stderr)
