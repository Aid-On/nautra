# Nautra

A container engine for WebAssembly: build, ship and run `.wasm` workloads with
exactly the grants the `run` line names. Almide 0.70 for everything that
decides; Rust for the engine (wasmtime). Design decisions:
[docs/adr/](docs/adr/) — read them before proposing a change in direction.
ADR-0001 is the foundation; ADR-0002 is how agents run here and the order
that work is done in.

## Layout

| | |
|---|---|
| `src/main.almd`, `help.almd` | command dispatch, one function per command |
| `src/grants.almd` | the `run` options: what a workload is given |
| `src/reference.almd` | image names `[registry/]name[:tag][@digest]` |
| `src/oci.almd` | OCI wire types, with their wire names |
| `src/store.almd` | the local store, an OCI image layout |
| `src/image.almd` | Nautrafile, building an image, what an image asks for |
| `src/registry.almd` | OCI Distribution: pull, push, authentication |
| `src/runs.almd` | background runs: the record, the log, stop and remove |
| `src/engine.almd` | every `@extern` into the native side |
| `native/engine.rs` | one run applied to wasmtime; the guest's view of the host, built from the request |
| `native/engine/outbound.rs` | outbound `wasi:http`: destinations, secrets, the audit record |
| `native/engine/serve.rs` | an HTTP component served on a published address (`-p`) |
| `native/engine/spawn.rs` | `run -d`: nautra started once more with no terminal |
| `native/engine/inspect.rs` | what a `.wasm` file is, without compiling it |
| `native/engine/upload.rs` | stopgap: a byte-exact request body (see ADR-0001 §3) |
| `examples/` | guest programs in Almide, built to `target/wasm` |
| `ee/` | Nautra Enterprise; a different licence. Core never imports it |

## Build and verify

Native builds need rustc 1.97 or newer on PATH.

```bash
bash scripts/install-almide.sh                          # pinned compiler -> .tools/almide
.tools/almide/almide check src/main.almd
.tools/almide/almide test                               # unit tests, under a second
.tools/almide/almide build src/main.almd -o target/nautra
bash scripts/build-examples.sh
python3 scripts/integration.py target/nautra
```

Push and pull are checked only when a scratch registry is named:

```bash
docker run -d --name nautra-test-registry -p 127.0.0.1:5057:5000 registry:2
NAUTRA_TEST_REGISTRY=localhost:5057 python3 scripts/integration.py target/nautra
```

## Invariants

- A workload starts with nothing. A new capability is a new grant on `run`,
  never a default.
- Nothing an image says about itself opens anything. `[wants]` is displayed.
- A blob is checked against its digest wherever it is used: pulled, read,
  pushed, run.
- Text that came from an image or a registry is stripped of control
  characters before it is printed.
- A name without a registry is never sent anywhere.
- A secret's real value exists only in the host process: the guest holds a
  placeholder, and no record, log or run file holds the value.
- Policy stays in Almide. The native side applies a finished request.
- Every source file carries the SPDX header of its directory: `Apache-2.0`
  outside `ee/`, `LicenseRef-Nautra-Enterprise` inside.

## Almide notes

- A local binding named after a module (`option`, `list`, `string`) shadows
  it silently; do not name one that way.
- Passing a generic stdlib function by name (`list.map(rows, list.len)`) hits
  a compiler bug in 0.70.0-rc1 (almide/almide#4745); wrap it in a lambda.
- `http.set_body_bytes` on a request is lossy in 0.70.0-rc1 (the body crosses
  as text, almide/almide#4744); binary uploads use `engine.upload`.
- A call on a `String` parameter inside a `while` condition does not build in
  0.70.0-rc1 (almide/almide#4752); hoist it into a `let` before the loop.
