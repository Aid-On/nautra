# ADR-0003: On someone else's platform

Status: accepted, 2026-11-15

Nautra runs on a laptop and on a server. It should also run where agents are
hosted for you: Amazon Bedrock AgentCore Runtime, Cloudflare Containers and
its Sandbox SDK, and beside agents that are not Wasm at all, such as those
built with Cloudflare's Agents SDK. This records what those places ask for
and the two shapes nautra takes in them.

## 1. Two shapes, and no third

Every platform looked at so far wants one of two things from nautra.

**The whole agent.** The platform runs a container and sends it HTTP
requests. The container holds the nautra binary, a composition file and the
images it names, and starts with `nautra run -f nautra.toml`. The platform
isolates one session from another; nautra decides what the agent inside may
do with its secrets, its tools and the network, and keeps the record. The two
do not overlap.

**The tools alone.** The agent lives elsewhere — in the platform's own agent
framework, or as a native process — and calls nautra's tools over MCP. Nautra
runs no workload; it publishes a composition's tools on an address (§2).

A platform that fits neither (one that takes a Wasm module and runs it in its
own engine, such as Workers) is not a place nautra runs. Deploying an image
to such an engine would be a translation of nautra's grants into that
platform's, a different product from running nautra there, and is not decided
here.

## 2. Publishing tools: `[mcp]` in place of `[agent]`

A composition file with an `[mcp]` table and no `[agent]` publishes its tools
as an MCP server:

```toml
[mcp]
listen = "8000"            # PORT, or ADDRESS:PORT, as for -p
token = "MCP_TOKEN"        # the host variable holding the callers' bearer token
audit = "calls.jsonl"

[tools.search]
image = "ghcr.io/you/search:v1"
net = ["api.example.com:443"]
```

`nautra run -f FILE` serves it until stopped. This is ADR-0002's step 6, and
it is `run`, not a new verb: the file says what is run.

- **The same tools.** A call is what it is inside a run: arguments checked
  against the schema, `before` checks first, a fresh instance under the
  tool's own grants, budgets counted, the call recorded in the audit. Only
  the caller differs.
- **Who may call is never left unsaid.** Whoever reaches the address can
  call the tools. On this machine's loopback that is the people on this
  machine. On any other address the file must name a token, or say
  `open = true` — which is for a platform that authenticates in front of
  nautra, and is the author's statement that one does.
- **Requests are JSON-RPC over `POST /mcp`, answered whole.** No session, no
  stream: a tool call here is a run that ends.
- **No finish checks and no record.** There is no run that finishes, and
  requests from outside cannot be replayed. A file that asks for either is
  refused.
- **The file is the whole line.** No other option or argument is taken with
  it; there is no workload for one to belong to.

Stdio, which ADR-0002 §7 named for a native agent on the same machine, is not
built: an agent that can start a process can reach a loopback port.

## 3. What the platforms ask for

| | architecture | shape | where the request arrives |
|---|---|---|---|
| Amazon Bedrock AgentCore Runtime | linux/arm64 | whole agent | `POST /invocations` and `GET /ping` on 8080; or MCP on 8000 |
| Cloudflare Containers, Sandbox SDK | linux/amd64 | whole agent, or tools | whatever port the Worker in front forwards to |
| Cloudflare Agents SDK, other frameworks | — | tools | MCP over HTTP |

So nautra is built and tested on both Linux architectures, and released for
both.

Native tools and checks need the host kernel's Landlock, seccomp and
namespaces. Inside a platform's container they may not be there; an entry the
sandbox cannot hold is refused, as everywhere (ADR-0002 §7), and everything
Wasm still works.

## Not decided yet

- **The request shape of a platform.** AgentCore's HTTP contract wants two
  paths the agent would otherwise have to implement itself. Whether nautra
  answers `/ping` and hands the body of `/invocations` to the workload, so an
  agent need not be written for one platform, is the next decision.
- **Signed model requests.** A secret here is a value put on a request for
  the hosts it is bound to. A model endpoint that wants each request signed
  (AWS SigV4) does not fit that; either an API key is used or the host signs.
- **Release artifacts**: binaries for both architectures and a container
  image of nautra itself.

None of this has been run on the platforms themselves yet. What is tested is
nautra's side of each contract.
