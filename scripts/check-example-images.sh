#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Builds the images in examples/platforms on the released base image, starts
# each, and asks it what a platform would. Needs docker and the example
# guests (scripts/build-examples.sh). The images are built for this machine's
# architecture; what a platform requires is in each Dockerfile.
set -euo pipefail
cd "$(dirname "$0")/.."

up() {
  for _ in $(seq 1 60); do
    curl -sf -o /dev/null -m 2 "http://127.0.0.1:$1/ping" && return 0
    sleep 0.5
  done
  docker logs "$2" >&2
  return 1
}

cleanup() { docker rm -f nautra-check-agentcore nautra-check-cloudflare >/dev/null 2>&1 || true; }
trap cleanup EXIT
cleanup

docker build -q -f examples/platforms/agentcore/Dockerfile -t nautra-check-agentcore . >/dev/null
docker build -q -f examples/platforms/cloudflare/Dockerfile -t nautra-check-cloudflare . >/dev/null

docker run -d --name nautra-check-agentcore -p 127.0.0.1:18080:8080 -e MODEL_KEY=k nautra-check-agentcore >/dev/null
docker run -d --name nautra-check-cloudflare -p 127.0.0.1:18000:8000 -e MCP_TOKEN=t nautra-check-cloudflare >/dev/null
up 18080 nautra-check-agentcore
up 18000 nautra-check-cloudflare

# An agent with no task ends with an error: the run happened, and the reply says why.
said=$(curl -s -m 60 -X POST -d '' http://127.0.0.1:18080/invocations)
case "$said" in *"give the agent a task"*) echo "ok    agentcore: a request is a run of the agent" ;; *) echo "FAIL  agentcore: $said"; exit 1 ;; esac

code=$(curl -s -m 5 -o /dev/null -w '%{http_code}' -X POST -d '{}' http://127.0.0.1:18000/mcp)
test "$code" = 401 && echo "ok    cloudflare: a caller without the token is refused" || { echo "FAIL  cloudflare: no token got $code"; exit 1; }
said=$(curl -s -m 60 -H 'Authorization: Bearer t' -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"read_file","arguments":{"path":"notes/tide.txt"}}}' http://127.0.0.1:18000/mcp)
case "$said" in *"the tide is in"*) echo "ok    cloudflare: a tool is called over MCP" ;; *) echo "FAIL  cloudflare: $said"; exit 1 ;; esac
