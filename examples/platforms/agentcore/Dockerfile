# SPDX-License-Identifier: Apache-2.0
# An agent for Amazon Bedrock AgentCore Runtime, which runs a linux/arm64
# container and sends it POST /invocations and GET /ping on port 8080.
#
# From the repository root, after `bash scripts/build-examples.sh`:
#
#   docker buildx build --platform linux/arm64 \
#     -f examples/platforms/agentcore/Dockerfile -t my-agent .
#
# The base image is a released nautra; name the version you mean.
FROM ghcr.io/aid-on/nautra:0.1.0

COPY --chown=nautra examples/platforms/agentcore/nautra.toml /home/nautra/agent/nautra.toml
COPY --chown=nautra target/wasm/agent.wasm target/wasm/read_file.wasm target/wasm/upper.wasm /home/nautra/agent/
COPY --chown=nautra examples/agent/workspace /home/nautra/agent/workspace

EXPOSE 8080
CMD ["run", "-f", "/home/nautra/agent/nautra.toml"]
