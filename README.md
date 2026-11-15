<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./docs/assets/brand/nautra-cover-dark.png">
    <img src="./docs/assets/brand/nautra-cover.png" alt="Nautra" width="720">
  </picture>
</p>

<p align="center"><strong>Build, ship and run WebAssembly workloads — with exactly what you grant them.</strong></p>

<p align="center">
  <a href="https://github.com/Aid-On/nautra/actions/workflows/ci.yml"><img src="https://github.com/Aid-On/nautra/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="./LICENSING.md"><img src="https://img.shields.io/badge/license-Apache--2.0%20%2B%20Enterprise-6447f0.svg" alt="License: Apache-2.0 + Nautra Enterprise"></a>
  <img src="https://img.shields.io/badge/status-pre--alpha-c679fc.svg" alt="Status: pre-alpha">
</p>

<p align="center">
  <a href="#quick-start">Quick start</a> ·
  <a href="#grants">Grants</a> ·
  <a href="#services">Services</a> ·
  <a href="#agents-and-tools">Agents</a> ·
  <a href="#background-runs">Background</a> ·
  <a href="#images">Images</a> ·
  <a href="#registries">Registries</a> ·
  <a href="#project-status">Status</a> ·
  <a href="./docs/adr/">Design</a> ·
  <a href="./LICENSING.md">Licensing</a>
</p>

<p align="center">
  <a href="README.md"><img alt="English" src="https://img.shields.io/badge/English-24292f?style=flat-square"></a>
  <a href="README.ja.md"><img alt="日本語" src="https://img.shields.io/badge/%E6%97%A5%E6%9C%AC%E8%AA%9E-d0d7de?style=flat-square"></a>
</p>

```text
$ nautra build -t upper:v1 .
upper:v1  sha256:802fb65f45b0acc4f189fca1e7d3deb22aadf4f363c035a0bbdbdcc1969c04b5

$ nautra run upper:v1
nautra: this image asks for grants this run does not give: -v <dir>:/data  --allow-net api.example.com:443  -e API_TOKEN
Error: fs.read_text("/data/in.txt"): Bad file descriptor (os error 9)

$ nautra run -v ./data:/data upper:v1
copied 5 characters from /data/in.txt to /data/out.txt
```

Nautra is to WebAssembly what a container engine is to Linux containers: it
packages a `.wasm` binary as an image, moves it through any OCI registry, and
runs it. The difference is the starting point. A container starts with a
whole filesystem and a network and is then restricted; a Wasm workload starts
with **nothing** — no directory, no network, no environment, a closed stdin —
and gets only what the `run` line names. There is no daemon, no guest kernel
and no root.

Nautra is developed by [Aid-On](https://github.com/Aid-On) and written in
[Almide](https://github.com/almide/almide), on [wasmtime](https://wasmtime.dev).
It grew out of [Porta](https://github.com/almide/porta), which confines native
processes with the host kernel; Nautra's unit is the Wasm binary.

## Quick start

Build from source (a release binary is not published yet). You need Rust
1.97 or newer.

```bash
git clone https://github.com/Aid-On/nautra && cd nautra
bash scripts/install-almide.sh                       # the pinned Almide compiler
.tools/almide/almide build src/main.almd -o target/nautra
bash scripts/build-examples.sh                       # example guests, to target/wasm
```

Run a `.wasm` file directly, or package it and run the image:

```bash
target/nautra run target/wasm/hello.wasm tide        # hello, tide
target/nautra build -t hello:v1 target/wasm/hello.wasm
target/nautra run hello:v1 tide
target/nautra images
```

Core modules (WASI preview 1) and components (WASI 0.2 and 0.3) all run; the
binary's header decides which.

## Grants

Each option opens one thing. Options come before the image; everything after
it belongs to the workload.

| Option | Opens |
|---|---|
| `-v HOST:GUEST[:ro]` | one directory, where the guest sees it; `:ro` for reads only |
| `-e NAME=VALUE`, `-e NAME` | one variable; a bare name copies the host's value |
| `--allow-net HOST:PORT` | one destination; also `HOST`, `*:PORT`, or `*` for all |
| `--secret NAME@HOST` | a host variable for HTTP requests to `HOST`; the workload holds a placeholder, never the value |
| `--audit FILE` | nothing — it appends every network decision of the run to `FILE` |
| `-p [ADDRESS:]PORT` | where an HTTP service listens; this machine only unless an address is named |
| `-i` | this terminal's stdin |
| `-m 64m` | a memory ceiling |
| `--timeout 30s` | a deadline (exit 124) |
| `--fuel N` | a ceiling on executed instructions (exit 137) |

```text
$ nautra run target/wasm/dial.wasm 127.0.0.1 8080
Error: tcp_connect(127.0.0.1:8080): permission denied

$ nautra run --allow-net 127.0.0.1:8080 target/wasm/dial.wasm 127.0.0.1 8080
connected to 127.0.0.1:8080
```

A network grant covers sockets and outbound HTTP (`wasi:http`) alike. For
sockets, a host name is resolved once, before the workload starts, so the
workload cannot steer where a granted name points. For HTTP, the host named in
the URL is matched against the grant, and a request anywhere else is refused
before a connection is made. A mount cannot be climbed out of with `..` or a
symlink.

### Secrets

A workload that calls an API needs a key, and a workload that holds a key can
send it anywhere it can reach. `--secret` separates the two:

```text
$ export API_TOKEN=real-token
$ nautra run --secret API_TOKEN@api.example.com app.wasm
```

Inside, `API_TOKEN` is a placeholder such as `nautra-secret-3f9a…`. The
workload sends it in whatever header its client uses; on a request to
`api.example.com` nautra replaces it with the real value, and a request that
carries it anywhere else is refused. The grant also opens HTTPS to that host,
and nothing more. A host the secret is bound to is trusted with it.

`--audit FILE` appends one line of JSON per network decision — each HTTP
request with its destination, path, status and the names of the secrets it
carried, each refusal with its reason, each socket connection. It never
records a header value, a query string or a secret.

The workload's own exit code is passed through. `125` means nautra refused
or could not start the run.

## Services

A component that exports an HTTP handler (`wasi:http`, 0.2 or 0.3) is a
service. It has no port of its own — it exports a function, and the host
decides where requests come from — so `-p` takes one port, not a mapping:

```text
$ nautra run -p 8080 target/wasm/web.wasm
nautra: serving on http://127.0.0.1:8080

$ curl http://127.0.0.1:8080/tide/pool
GET /tide/pool from a wasm component
```

`-p 8080` listens on this machine only; `-p 0.0.0.0:8080` is the explicit way
to accept connections from elsewhere. Requests are handled side by side,
`--timeout` is a deadline per request, and the service is given the same
grants as any run: nothing by default. It stops on Ctrl-C or SIGTERM.

## Agents and tools

An agent here is an ordinary component whose every dealing with the world is
an HTTP request, and nautra decides what each URL means. A model is a granted
host with a bound secret. Tools are one [MCP](https://modelcontextprotocol.io)
server at `http://tools.nautra/` — a name that never leaves nautra — and each
tool behind it is another image, started fresh for every call with grants of
its own.

Which tools a workload has is written in a file:

```toml
# nautra.toml
[agent]
image = "ghcr.io/you/agent:v1"
env = ["MODEL_URL=https://api.example.com/v1/chat/completions"]
secrets = ["MODEL_KEY@api.example.com"]     # the agent holds a placeholder
timeout = "2m"
max_tool_calls = 16

[tools.read_file]
image = "ghcr.io/you/fs-tools@sha256:…"     # pinned by naming its digest
description = "Read a file from the workspace."
input = { type = "object", properties = { path = { type = "string" } }, required = ["path"] }
mounts = ["./workspace:/work:ro"]           # this tool's, not the agent's

[tools.upper]
image = "upper:v1"                           # no grants: it can only compute
```

```text
$ nautra run -f nautra.toml "What does the note say?"
The note says: THE TIDE IS IN
```

The file says nothing a `run` line could not: each entry's `mounts`, `net`,
`env`, `secrets`, `timeout`, `memory` and `fuel` are the options of the same
names, read by the same parser. Paths in it are relative to the file. Options
on the command line add to the `[agent]` entry.

A tool is any command image: it gets the call's arguments as JSON on stdin
and its stdout is the result, so it can be tried by hand.

```text
$ echo '{"text": "tide"}' | nautra run -i upper:v1
TIDE
```

Arguments are checked against `input` on the host before the tool is started,
and a refusal says what was wrong, so a model can correct the call. `max_calls`
on a tool and `max_tool_calls` on the agent are ceilings; past them a call
returns an error result and nothing runs. With `--audit`, every tool call is
recorded beside the network decisions, the tools' own included.

[`examples/agent`](examples/agent) is a complete agent in Almide, about a
hundred lines, with two tools.

### Record, replay, resume

`--record FILE` writes down every exchange at a run's boundary, in order:
each outbound HTTP request with its response, each tool call with its result.

```text
$ nautra run --record run.record -f nautra.toml "What does the note say?"
The note says: THE TIDE IS IN

$ nautra inspect run.record
record    run.record
wasm      sha256:…
run       -f nautra.toml What does the note say?
exchanges 5 (3 http, 2 tool)
state     finished, exit code 0

$ nautra replay run.record
The note says: THE TIDE IS IN
nautra: the run matches its record: 5 exchange(s) replayed, nothing sent
```

A replay runs the workload again and answers every exchange from the record:
no model is called, no tool is started, no key is needed. If the workload asks
for anything other than what the record has next, the run has left its record
and ends with exit code 3 — so a replay is also a check that the same binary
still makes the same decisions.

A run that stopped part-way is continued with `nautra run --resume FILE`: the
recorded exchanges are replayed, and the run goes on for real from there,
appending to the record. Each exchange is written in two steps, the request
before it is sent and the answer after, so a run that stopped in between is
known to have stopped there. That exchange may or may not have taken effect;
it is not repeated, and the run is not resumed past it.

A record is tied to the exact binary it was made with and is replayed from
the directory it was made in. It holds what the workload received, so it is as
private as the run was; it never holds a secret, because request headers are
not recorded. Under `--record`, a streamed response reaches the workload all
at once. The clock and random numbers are not recorded, so a workload whose
requests depend on them will leave its record.

## Background runs

`-d` starts a run in the background and prints its id. There is no daemon
behind it: the run is one more nautra process with no terminal, and what
nautra knows about it is a directory under `~/.nautra/runs`.

```text
$ nautra run -d --name web -p 8080 target/wasm/web.wasm
5e834c6e4266

$ nautra ps
ID            NAME  IMAGE                 STATUS   PORT            STARTED
5e834c6e4266  web   target/wasm/web.wasm  running  127.0.0.1:8080  2026-11-15T07:27:31Z

$ nautra logs web
nautra: serving on http://127.0.0.1:8080

$ nautra stop web
web
```

| | |
|---|---|
| `nautra run -d [--name NAME] [--rm] …` | start in the background; `--rm` forgets the run when it ends |
| `nautra ps [-a]` | running runs; `-a` adds the ones that ended, with how |
| `nautra logs [-f] RUN` | what it wrote to stdout and stderr; `-f` follows until it ends |
| `nautra stop [-t 10s] RUN…` | SIGTERM, then SIGKILL after the wait |
| `nautra rm [-f] RUN…` | forget an ended run and its log; `-f` stops it first |

A `RUN` is a name, an id, or the start of an id. A run that fails at once —
a port already taken, a missing mount — is reported by `run -d` itself, with
its exit code, instead of being left to be found in `ps`. A background run
ends with the machine; nothing restarts it.

## Images

An image is a `.wasm` binary plus a description. Describe it in a `Nautrafile`:

```toml
wasm = "app.wasm"
description = "Uppercases /data/in.txt into /data/out.txt"

[run]
args = ["/data/in.txt", "/data/out.txt"]

[run.env]
LOG_LEVEL = "info"

[wants]                     # what a run needs to be granted
mounts = ["/data"]
net = ["api.example.com:443"]
env = ["API_TOKEN"]
```

`[wants]` is a statement by the image's author, not a permission. Nautra
shows it — on `inspect`, and on a `run` that does not grant it — and never
opens anything because an image asked.

```text
$ nautra inspect upper:v1
name      upper:v1
digest    sha256:802fb65f45b0acc4f189fca1e7d3deb22aadf4f363c035a0bbdbdcc1969c04b5
wasm      sha256:e45630d64ca18182ead688724388cabc2a5958fb6fc1e835fd95ef2a8eb80d03
kind      module
size      19.3 kB
reach     files, environment
about     Uppercases /data/in.txt into /data/out.txt
args      /data/in.txt, /data/out.txt
env       LOG_LEVEL=info
asks for  -v <dir>:/data, --allow-net api.example.com:443, -e API_TOKEN
```

`reach` is read from the binary's imports: the outer bound of what it can
ever use. A binary that does not import sockets cannot reach the network,
whatever it is granted.

Images are stored as a standard [OCI image layout](https://github.com/opencontainers/image-spec/blob/main/image-layout.md)
in `~/.nautra/store` (`$NAUTRA_HOME` to move it), in the shape of the CNCF
[Wasm OCI Artifact](https://tag-runtime.cncf.io/wgs/wasm/deliverables/wasm-oci-artifact/)
layout. Every blob is checked against its digest each time it is read, so a
binary that changed on disk is refused, not run. With `SOURCE_DATE_EPOCH` set,
the same inputs build the same digest.

| | |
|---|---|
| `nautra build [-t NAME] [-f Nautrafile] [PATH]` | package; `PATH` is a directory with a Nautrafile, or a `.wasm` file |
| `nautra images` | list |
| `nautra inspect IMAGE\|FILE.wasm` | what it is, what it asks for, what it can reach |
| `nautra tag SOURCE NAME` | add a name |
| `nautra rmi IMAGE…` | remove names, and blobs nothing refers to any more |

## Registries

```bash
nautra build -t ghcr.io/you/app:v1 .
nautra push ghcr.io/you/app:v1
nautra run ghcr.io/you/app:v1          # pulled on first use
nautra pull ghcr.io/you/app@sha256:…   # pinned to a digest
```

Any registry that speaks the OCI Distribution API works. A name without a
registry (`app:v1`) is local only — nautra has no default registry, so it
never contacts one you did not name. Everything pulled is hashed and compared
with the digest that named it before it enters the store, and an image with
no Wasm layer (a Linux container image) is refused.

Logins are read from `NAUTRA_USERNAME` and `NAUTRA_PASSWORD` (a registry
token works as the password).

## Project status

**Pre-alpha.** What is above works and is covered by `scripts/integration.py`
on macOS and, in CI, on Linux. WASI 0.3 and its `wasi:http` are still marked
experimental in wasmtime, and services inherit that. Push and pull are tested against a local
`registry:2`, and have not yet been verified against public registries.

Agents are being built in the order [ADR-0002](docs/adr/0002-agents.md)
gives. Secrets, the audit of decisions, tools, and record / replay / resume
are in; checks before a tool call and before finishing, delegation to other
agents, remote MCP servers, and confining native commands are not.

Not there yet:

- restart policies and starting runs again after a reboot
- TLS on a published port, and HTTP/2
- several workloads together (compose) and networking between them
- signing, provenance and policy on what may be pulled
- credential helpers, a compiled-code cache, release binaries

## Editions and licence

Nautra is open core:

| | Community Edition | Nautra Enterprise |
|---|---|---|
| Source | everything outside [`ee/`](ee/) | adds the contents of [`ee/`](ee/) |
| Licence | [Apache License 2.0](LICENSE) | [Nautra Enterprise License](ee/LICENSE), with a subscription from Aid-On |

Everything described in this README is Community Edition. The split, and the
rules that keep it clean, are in [LICENSING.md](LICENSING.md). The name and
logo are covered by [TRADEMARKS.md](TRADEMARKS.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Contributions are accepted under the
[Contributor License Agreement](CLA.md). Report vulnerabilities privately, as
described in [SECURITY.md](SECURITY.md).

## The name

*Nautra* is a coined word built on *naut-*, from *nautilus* — the animal in
the logo, which lives in the outermost chamber of a shell it seals off behind
itself.
