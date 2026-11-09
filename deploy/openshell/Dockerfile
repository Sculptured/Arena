# The sandbox image: OpenShell's Ubuntu base plus this machine's Claude Code binary.
# Build from a folder holding a copy of the binary:  cp "$(readlink -f "$(which claude)")" claude
#   docker build -t openshell-arena-claude:$(claude --version | cut -d' ' -f1) .
FROM nvcr.io/nvidia/base/ubuntu:24.04
USER root
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --chmod=0755 claude /usr/local/bin/claude
