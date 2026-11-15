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
images it names, and starts with `nautra run -f nautra.toml`; the file
publishes the agent with `[invoke]` (§3). The platform
isolates one session from another; nautra decides what the agent inside may
do with its secrets, its tools and the network, and keeps the record. The two
do not overlap.

**The tools alone.** The agent lives elsewhere — in the platform's own agent
framework, or as a native process — and calls nautra's tools over MCP. Nautra
runs no workload; it publishes a composition's tools on an address (§2).

Both are a table in the composition file that says where to listen and who
may call, and both are `run -f`.

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

## 3. Publishing an agent: `[invoke]`

An agent here is a workload that ends: it is given a task, works, writes its
answer and exits. A platform wants a server. `[invoke]` beside `[agent]`
makes one out of the other without the agent knowing:

```toml
[agent]
image = "agent.wasm"
secrets = ["MODEL_KEY@api.example.com"]

[invoke]
listen = "0.0.0.0:8080"
open = true                # the platform in front authenticates
```

- **Each POST is one whole run.** The request body is the run's stdin; what
  it writes to stdout is the response, as it is (`application/json` when it
  is JSON). A run that does not end with 0 is a 500 whose body says why. The
  agent's tools, checks and budgets are those of any run, counted afresh.
- **Nothing is kept between requests.** A run starts from the file, so one
  request cannot leave anything for the next. State that must last belongs
  in a mount or behind a tool.
- **Any path.** The platform chooses it (`/invocations` on AgentCore); the
  workload is not told.
- **`GET /ping` is nautra's**, on this server and on `[mcp]`'s: it answers
  `{"status": "Healthy"}` without a token and without running anything. It
  says the process is up, which is what a platform's health check asks.
- **Who may call** is said the same way as for `[mcp]`, and for the same
  reason.
- **No record**, as for any published service.

An agent written to read its task from stdin when it has no arguments runs
the same from a shell, as a tool of another agent, and behind `[invoke]`.

The body is not interpreted. A platform's own envelope (`{"prompt": …}`) is
the agent's to read, as its payload always was.

## 4. What the platforms ask for

| | architecture | shape | where the request arrives |
|---|---|---|---|
| Amazon Bedrock AgentCore Runtime | linux/arm64 | whole agent (`[invoke]`), or tools (`[mcp]`) | `POST /invocations` and `GET /ping` on 8080; or `POST /mcp` on 8000 |
| Cloudflare Containers, Sandbox SDK | linux/amd64 | whole agent, or tools | whatever port the Worker in front forwards to |
| Cloudflare Agents SDK, other frameworks | — | tools | MCP over HTTP |

So nautra is built and tested on both Linux architectures, and released for
both.

Native tools and checks need the host kernel's Landlock, seccomp and
namespaces. Inside a platform's container they may not be there; an entry the
sandbox cannot hold is refused, as everywhere (ADR-0002 §7), and everything
Wasm still works.

## Not decided yet

- **Streaming.** A response is the whole of what a run wrote. A platform
  that takes server-sent events gets none.
- **Sessions.** A platform that names a session with a header
  (`X-Amzn-Bedrock-AgentCore-Runtime-Session-Id`) is not listened to: every
  request is a fresh run.
- **Signed model requests.** A secret here is a value put on a request for
  the hosts it is bound to. A model endpoint that wants each request signed
  (AWS SigV4) does not fit that; either an API key is used or the host signs.
- **Release artifacts**: binaries for both architectures and a container
  image of nautra itself.

None of this has been run on the platforms themselves yet. What is tested is
nautra's side of each contract.
