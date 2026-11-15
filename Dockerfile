# SPDX-License-Identifier: Apache-2.0
# The nautra image: the binary a release built and tested, and nothing else
# that runs. It is built from that binary, not from source, so what is in the
# image is what the integration suite ran.
#
#   docker run --rm -v "$PWD:/w:ro" ghcr.io/aid-on/nautra run /w/hello.wasm
#
# A platform that runs a container per agent (Amazon Bedrock AgentCore
# Runtime wants linux/arm64, Cloudflare Containers linux/amd64) starts from
# this image and adds a composition file and the images it names.
FROM debian:trixie-slim

# Certificates for outbound HTTPS and for pulling from a registry. The base
# has none.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Not root: nothing nautra does needs it, and the sandbox for native commands
# refuses to run as root.
RUN useradd --create-home --uid 10001 nautra

ARG TARGETARCH
COPY dist/image/${TARGETARCH}/nautra /usr/local/bin/nautra
COPY LICENSE NOTICE /usr/share/doc/nautra/

USER nautra
WORKDIR /home/nautra
ENTRYPOINT ["nautra"]
CMD ["help"]
