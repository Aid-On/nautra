# ADR-0001: Foundations

Status: accepted (2026-11-15)

Nautra takes the container engine's job — build, ship, run — and does it for
WebAssembly. It continues the line [Porta](https://github.com/almide/porta)
started: Porta's roadmap had image distribution, compose and networking on
hold while it concentrated on confining native processes and running agents.
Nautra makes those the product. These are the decisions everything else is
built on.

## 1. The unit is one Wasm binary, in a standard OCI image

An image is an OCI image manifest whose single layer is a WebAssembly binary
(`application/wasm`) and whose config is `application/vnd.wasm.config.v0+json`,
the shape of the CNCF Wasm OCI Artifact layout. The local store is an OCI
image layout.

Nautra's own data — the default arguments and environment, and what the image
asks to be granted — travels in one manifest annotation, `dev.nautra.config`.

*Why:* an image nautra builds is stored by any OCI registry and readable by
other Wasm tooling, and an image other tooling published is runnable here.
Porta's on-hold design used its own tar media type; that would have made an
island. An annotation is the extension point the format provides, so the
artifact stays standard for everyone who ignores it.

## 2. A workload starts with nothing, and only `run` grants

No directory, no network, no environment, a closed stdin. Each `run` option
opens one thing. A restriction is never the default being narrowed; it is the
absence of a grant.

What an image declares (`[wants]`) is shown to the person running it and is
never applied. An image cannot grant itself anything.

*Why:* this is what the Wasm sandbox already is; the engine's job is to not
give it away. A container image's `EXPOSE`, `VOLUME` and `USER` blur
description and effect. Keeping them apart means the `run` line is the whole
policy of a run, readable in one place.

## 3. Almide decides, Rust applies

Everything that decides what a run may do, what an image is, and how a
registry is spoken to is Almide (`src/`). The native side (`native/`) is the
engine: it receives one finished request and applies it to wasmtime. It holds
no policy.

*Why:* Aid-On's domain is Wasm and Almide; the part worth reading and testing
is the policy, and Almide's tests run it in milliseconds. The native surface
stays small enough to audit: wasmtime, a binary-format reader for `inspect`,
a TOML reader, and one stopgap (below).

*Stopgap:* Almide 0.70's http client carries a request body as text, so a
binary body does not survive it. Blob uploads go through
`native/engine/upload.rs` until the stdlib client can send bytes
([almide/almide#4744](https://github.com/almide/almide/issues/4744)).

## 4. No default registry

A name without a registry is local. `pull` and `push` need the registry
written out.

*Why:* a default registry decides where an unqualified name resolves, which
is both a supply-chain risk (the wrong `app` from a public namespace) and a
commercial dependency. Nautra contacts only what was named.

## 5. Content is checked where it is used

A blob is hashed against its digest when it is pulled, when it is read from
the store, before it is pushed and before it is run. Text that came from an
image is stripped of control characters before it is printed.

*Why:* the store is files on disk and a registry is someone else's server.
"What runs is what the digest says" has to hold at the moment of running, not
only at the moment of download.

## 6. No daemon

`nautra run` is one process that runs one workload and exits with its exit
code. Long-running and multi-workload operation, when it comes, is built on
that — supervised by the tool the host already has, or by a nautra supervisor
that is itself an ordinary process.

*Why:* a privileged always-on daemon is the largest part of a container
engine's attack surface and the reason it needs root. A Wasm workload needs
neither.

## 7. A service is published, not mapped

A component that exports a `wasi:http` handler is served with `-p PORT`. The
option takes one port, on this machine only unless an address is written
(`-p 0.0.0.0:8080`). There is no `HOST:CONTAINER` mapping and no `EXPOSE`.

*Why:* a Wasm HTTP service exports a function; it never binds a port, so
there is nothing inside to map to. Listening on every interface by default is
a known way container ports end up reachable from a network nobody meant, so
the wider reach is the one that has to be spelled out.

## 8. Open core, split by who needs it

Capabilities an individual or a single team needs to build, ship and run Wasm
workloads are Community Edition (Apache-2.0). Capabilities that exist because
an organisation operates nautra at scale — organisation-wide policy on what
may be pulled and granted, audit, identity integration, fleet management —
belong in `ee/`. Core never depends on `ee/`. See [LICENSING.md](../../LICENSING.md).
