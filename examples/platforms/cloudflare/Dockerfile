# SPDX-License-Identifier: Apache-2.0
# Tools for an agent on Cloudflare: a linux/amd64 container (Cloudflare
# Containers) that publishes a composition's tools as an MCP server on port
# 8000. A Worker in front forwards to it; an agent built with the Agents SDK
# connects to that Worker as to any remote MCP server.
#
# From the repository root, after `bash scripts/build-examples.sh`:
#
#   docker buildx build --platform linux/amd64 \
#     -f examples/platforms/cloudflare/Dockerfile -t my-tools .
#
# Use a released version of the base image in place of `latest`.
FROM ghcr.io/aid-on/nautra:latest

COPY --chown=nautra examples/platforms/cloudflare/nautra.toml /home/nautra/tools/nautra.toml
COPY --chown=nautra target/wasm/read_file.wasm target/wasm/upper.wasm /home/nautra/tools/
COPY --chown=nautra examples/agent/workspace /home/nautra/tools/workspace

EXPOSE 8000
CMD ["run", "-f", "/home/nautra/tools/nautra.toml"]
