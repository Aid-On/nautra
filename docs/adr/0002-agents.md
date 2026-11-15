# ADR-0002: Agents

Status: accepted (2026-11-15), §7 revised the same day. Steps 1 to 5 of the
order below are implemented; step 6 is not.

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

**Checks.** A check is run fresh under its own grants, told what it is asked
about as JSON on stdin (`{"event": "tool", "tool", "arguments"}` or
`{"event": "finish"}`), and answers by its exit code; its output is the
reason. A check `before` a tool refuses the call without the tool running. A
`finish` check runs after the workload ends with exit 0; a refusal makes the
run's exit code 4. A workload ending is a claim, and the check accepts it or
does not. The same finish checks are offered to the workload as the tool
`verify`, so an agent can learn what is missing while it can still act —
feedback without a protocol of its own.

**Delegation.** A tool entry with `agent = "other.toml"` is that whole
composition, called with one argument, `task`. It shares nothing with its
caller. Depth is limited to four, which also stops a file that names itself.

**Remote MCP.** Not an engine feature. A remote server is reached by a tool
image that speaks MCP to it (`examples/tools/mcp_call.almd`), so the server
is a granted host with a bound secret like any other destination, its calls
are in the audit as that tool's requests, and under a record its results are
tool results.

## 5. The record

`--audit FILE` appends one JSON object per decision at the boundary: each
outbound HTTP request (destination, path, which secrets it carried, the
status or the refusal and its reason) and each socket use. It never holds a
header value, a query string or a secret.

`--record FILE` writes what a run needs to be reproduced: every exchange at
its boundary, in order — an outbound HTTP request and its response, a tool
call and its result. `nautra replay FILE` runs the workload again and answers
each exchange from the record; no model is called and no tool is started. A
workload that asks for something other than what the record has next has left
its record, and the run ends with exit code 3. `nautra run --resume FILE`
replays what a stopped run had done and continues for real.

Three decisions shape it:

- **An exchange is two lines**, what was asked before it is carried out and
  what came back after. A run that died in between is known to have, and
  that exchange is never repeated: it may have taken effect.
- **What was asked is the method, the URL and a digest of the body.** Headers
  are not part of it, so no secret enters a record, and a replay needs none.
- **A record is tied to the binary** by its digest, and names the `run` line
  it was made with, so `replay` and `--resume` take only the file.

It is not specific to agents; any component that ends can be recorded. The
clock and randomness are not recorded yet, so a workload whose requests
depend on them leaves its record. A response is read whole before the
workload sees it, so streaming is lost under `--record`.

## 6. Verbs do not multiply

| Porta | Nautra |
|---|---|
| `agent <config> -- task` | `run` |
| `agent-check` | `inspect` on the composition |
| `agent-journal` | `inspect` on a record |
| `agent-resume`, `agent-replay` | `run --resume`, `replay` |
| `serve` (an agent's tools as an MCP server) | publishing an image's tools over MCP |
| `run <native command>`, `explain`, `--why`, `--snapshot` | stay Porta's; a native tool or check here is run through Porta (§7) |

## 7. Native commands are Porta's, reached through nautra's words

*Revised 2026-11-15. The first form of this section moved Porta's kernel
sandbox here as `nautra exec`. Porta continues as a sister project, so two
copies of that code would have to be kept equally strong; there is no `exec`.
Revised again the same day: Porta's sandbox became a package, and nautra
builds it in.*

Nautra confines Wasm, and that guarantee holds whatever the host is. A native
process can only be confined by the host kernel, as strongly as this platform
allows — that is Porta's work, and it stays Porta's.

Where an agent here needs a native program — git, a compiler, a test runner —
a tool or a check entry names it with `command = […]` in place of `image`, and
nautra confines it with Porta's sandbox. The entry's grants are written in
nautra's words and turned into one sandbox request (`src/native.almd`),
keeping nautra's stance: no network unless named, reads confined to the
mounts and the system directories unless `reads = "open"`. `porta = […]`
names, in Porta's words, what only the sandbox has a word for — ceilings and
further closures; a word that would open what the entry's own keys decide is
not taken.

Where Porta cannot hold a native process to what nautra's words would
promise, the entry is refused rather than run more openly: a mount has no
guest path (a native command sees a host directory where it is), and network
is granted by host name over HTTPS or by port, never by address and never by
both, since a run held to names leaves only through the proxy.

**Independence.** Nautra must be usable alone, and one nautra binary is
enough. Porta publishes its sandbox as an Almide package (`porta_sandbox`, the
`sandbox/` directory of its repository) and nautra builds it in as a
dependency, so there is one copy of that code, kept in Porta. Nautra does not
run the command in its own process: it starts itself once more
(`nautra __native ENTRY`), and that copy judges the entry again, applies the
request and waits, so the command's output is collected like any tool's.

The other direction — a native agent running under Porta, using nautra's
tools — is nautra publishing a composition's tools as an MCP server on
stdio. It is not built yet.

## 8. Porta's guest protocol is not carried over

"Everything Porta does" means its abilities, not its wire formats. The
stdin JSON protocol and the tool ABI are not implemented here; an agent
written for them is ported by making it speak MCP over HTTP.

## Order

1. Secrets and the network record. *(done)*
2. Tools as images, the composition file, and a sample agent in Almide. *(done)*
3. `--record`, `replay`, resume. *(done, without clock and randomness)*
4. Checks, delegation, remote MCP. *(done)*
5. Native commands as tools and checks, through Porta's sandbox, built in.
   *(done)*
6. Publishing a composition's tools as an MCP server, for native agents.

Organisation-wide concerns — collecting records centrally, policy on which
images may be pulled and which grants and secrets may be given, signing
records — belong in `ee/` (ADR-0001 §8).
