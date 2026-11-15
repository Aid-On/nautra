# ADR-0002: Agents

Status: accepted (2026-11-15). Implemented so far: §2 (secrets), §3 (tools),
§4 for one workload and its tools, and §5's `--audit`.

Nautra is to be an environment agents run in, and to do everything
[Porta](https://github.com/almide/porta) does. Porta grew its agent runtime
as a set of dedicated mechanisms: a JSON-lines guest protocol on stdin, a
length-prefixed tool ABI, an `agent.toml`, and a command for each stage
(`agent`, `agent-check`, `agent-journal`, `agent-resume`, `agent-replay`).
This records how the same abilities fit here with fewer concepts.

## 1. An agent is a component that speaks HTTP

Nothing about an agent is special to the engine. It is a Wasm component,
packaged and run like any other, whose every dealing with the world is an
outbound HTTP request. Nautra decides what each URL means:

| What the agent does | What it is here |
|---|---|
| calls a model | an HTTP request to a granted host; the API key is held by the host and put on requests to that host only (§2) |
| uses a tool | MCP over HTTP to a reserved local name; each tool is another image, started fresh per call with grants of its own (§3) |
| delegates to another agent | a tool whose image is an agent |
| uses a remote MCP server | an entry in the same tool list, granted explicitly |

*Why:* one boundary. Grants, budgets, the record and replay all attach to
the same place, and there is no agent-specific ABI to learn or to keep
compatible: a program with an MCP client runs here once it is compiled to
Wasm.

## 2. Secrets are bound to destinations, and the guest holds a placeholder

`--secret NAME@HOST[:PORT][,HOST…]` reads `NAME` from the host's environment.
The guest gets a meaningless placeholder under the same name. On an HTTP
request to a bound host, the placeholder in a header is replaced by the real
value; a placeholder headed anywhere else — another host, the URL itself, an
unencrypted connection to a host that is not this machine — refuses the
request. Binding a secret to a host also grants HTTP to that host, and only
HTTP: raw sockets to it stay closed.

*Why a placeholder, not a named header:* providers disagree on the header
(`x-api-key`, `Authorization: Bearer …`). With a placeholder the guest writes
the header its SDK already writes, and nautra needs no table of providers.

*What it does not protect against:* a bound host is trusted with the secret.
If that host reflects request headers back, the guest can read the reflection.

## 3. Tools are images

A tool is an image with a description and an input schema, run in a fresh
instance per call with exactly the grants its entry names — usually none. The
agent sees tools as one MCP server at a reserved local name. Arguments are
validated against the schema on the host, before the tool is started.

A tool is a command image: the call's arguments arrive as JSON on its stdin,
and what it writes to stdout is the result; a non-zero exit makes the result
an error carrying its stderr. That is the whole contract, so a tool can be
written in anything and tried from a shell. The reserved name is
`tools.nautra`; a request to it is answered inside nautra and never sent.

A failed or refused call comes back as an error *result*, not a protocol
error, and says what was wrong — the schema violations, the exhausted budget
— so that a model reading it can correct itself.

Because a tool is an image, pinning it is naming it by digest; no separate
pinning mechanism is needed.

## 4. One composition file

Which tools an agent has, each one's grants, and the budgets (calls per
tool, and in total) are declared in one file, `nautra.toml`, given to
`run -f`. An entry's grants are spelled as the `run` options of the same
names and are read by the same parser, so the file is another way to write a
`run` line, not a second policy language. The same format
describes several services that run together: an entry is an image, its
grants, and the name others reach it by. Checks that must pass before a tool
call or before the agent may finish are images too, named in the same file.

## 5. The record

`--audit FILE` appends one JSON object per decision at the boundary: each
outbound HTTP request (destination, path, which secrets it carried, the
status or the refusal and its reason) and each socket use. It never holds a
header value, a query string or a secret.

`--record FILE` will extend this to everything a run needs to be reproduced
— responses, tool results, clock and randomness — so that `nautra replay`
re-runs the guest with no model and no tool invoked, and an interrupted run
can be resumed. It is not specific to agents.

## 6. Verbs do not multiply

| Porta | Nautra |
|---|---|
| `agent <config> -- task` | `run` |
| `agent-check` | `inspect` on the composition |
| `agent-journal` | `inspect` on a record |
| `agent-resume`, `agent-replay` | `run --resume`, `replay` |
| `serve` (an agent's tools as an MCP server) | publishing an image's tools over MCP |
| `run <native command>`, `explain`, `--why`, `--snapshot` | `exec` (§7) |

## 7. Native commands are `exec`, not `run`

Confining a native process with the host kernel moves here from Porta as
`nautra exec`, taking the same grant options as `run`. It is a separate verb
because the guarantee is a different one: `run` is a Wasm sandbox that holds
whatever the host is, `exec` is as strong as this platform's kernel
facilities and says what it cannot enforce.

## 8. Porta's guest protocol is not carried over

"Everything Porta does" means its abilities, not its wire formats. The
stdin JSON protocol and the tool ABI are not implemented here; an agent
written for them is ported by making it speak MCP over HTTP.

## Order

1. Secrets and the network record. *(done)*
2. Tools as images, the composition file, and a sample agent in Almide. *(done)*
3. `--record`, `replay`, resume.
4. Checks, delegation, remote MCP.
5. `exec`.

Organisation-wide concerns — collecting records centrally, policy on which
images may be pulled and which grants and secrets may be given, signing
records — belong in `ee/` (ADR-0001 §8).
