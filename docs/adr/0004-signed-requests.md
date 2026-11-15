# ADR-0004: Requests the host authenticates

Status: accepted, 2026-11-16

A secret (ADR-0002) is a value: nautra puts it on a request where the guest
wrote its placeholder. Much of what an agent has to reach takes no such
value. AWS wants every request signed with a key that is never sent. Google
Cloud and Azure want a short-lived token minted from a key, or handed out by
the machine the code runs on. In each case there is nothing lasting for a
placeholder to stand for, so ADR-0003 left "signed model requests" open.

This decides the general thing — a destination whose requests nautra
authenticates as the host — and builds one way of doing it, AWS's signature,
because running an agent against Amazon Bedrock needs it first. The grant,
the engine and the audit are written for any way; only one file knows AWS.

## 1. A destination is authenticated to, not a value handed over

```toml
[agent]
image = "agent.wasm"
auth = ["sigv4@bedrock-runtime.us-east-1.amazonaws.com"]
```

On the command line, `--auth SCHEME@HOST[:PORT][,KEY=VALUE…]`.

- **The guest holds nothing**, not even a placeholder. It sends the request
  it would send to an open endpoint, and nautra authenticates it on the way
  out. So the same agent binary runs against any provider: what changes is
  a line of the file, not the workload.
- **Every request to the host is authenticated**, and only those. Naming the
  host is also the grant to send HTTP there, as for a secret, and for the
  same reason.
- **Over HTTPS only**, except to this machine's loopback.
- **The scheme is a word, and its parameters are its own.** `SCHEME` says
  how; `KEY=VALUE` pairs after the host are that scheme's words and no
  one else's. Nothing about the option is shaped by one provider.
- **One host to a grant.** Another host is another `--auth`.

The same key works in a tool's table, with the tool's own destinations. A
native command's requests are not authenticated for it (§6).

## 2. A scheme

A scheme is one way of authenticating a request: a signature over it, a
token fetched and put on it. It is three functions (`src/auth.almd`):

| | |
|---|---|
| `complete(host, params)` | the grant's words, checked, and filled in from the host name where they can be read off it |
| `ready(params)` | whether the host has what the scheme needs — said before the run starts |
| `answer(question)` | for one request: the headers to drop, and the headers to put on |

The question is the same for every scheme: the method, the host, the path,
the query, the headers the guest wrote, and — only for a scheme that asks —
the hash of the body. The answer is the same too. Because of that:

- **Nothing outside a scheme's file knows how it works.** The engine carries
  `scheme` and `params` without reading them; it has no word of any
  provider's in it. The option's parser reads a shape, not a meaning.
- **A scheme pays only for what it uses.** A signature covers the body, so
  the body is read whole and hashed before the request leaves, and one over
  32 MiB is refused. A scheme that only adds a token says so, and its
  requests stream as they always did.
- **A scheme owns its headers.** The headers it names are dropped before its
  own are put on, so a guest cannot bring credentials made elsewhere through
  a host nautra answers for.
- **Adding one is a file and three lines** (`src/authenticator.almd` lists
  the schemes). The option, the engine, the audit and the record do not
  change.

## 3. The first scheme: `sigv4`

AWS Signature Version 4 — named for the signature, not the company, because
it is also what most S3-compatible object stores ask for.

```text
sigv4@bedrock-runtime.us-east-1.amazonaws.com
sigv4@objects.example.com,region=auto,service=s3,env=STORE
```

- **`region` and `service` are read off an AWS host name** and need writing
  only where they cannot be: a host that is not AWS's, or an AWS one that
  does not say. A written word always wins. What is read is the shape AWS
  host names have (`SERVICE.REGION.amazonaws.com`, with a bucket in front,
  `dualstack`, `-fips`, a VPC endpoint; the global endpoints, which sign in
  `us-east-1`), not a table of services. The name a service signs under is
  its host's first label but for a few (`bedrock-runtime` signs as
  `bedrock`); those few are listed, and where the list is behind, the
  service's own refusal names the right word and `service=` says it. A host
  name that does not say is refused, never guessed at.
- **`env` is the prefix of the host variables holding the credentials**:
  `AWS` unless written, so `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and
  `AWS_SESSION_TOKEN` when the key is temporary. Two stores are signed for
  with two keys by giving one of them another prefix.
- **Signed**: the method, path and query as the guest wrote them, the whole
  body, `host` and the date, the session token, `content-type`, and every
  `x-amz-…` header the guest set (`x-amz-target` is how some services name
  the operation). Other headers travel unsigned, so a proxy that rewrites
  one does not break the signature.

Other tools were looked at for a form to share. They agree on the two words,
region and service, and on little else; most take them as two named options,
which is what they are here, and reading them off the host name is what
curl and AWS's own signing proxy do.

## 4. Where it happens

In nautra, in Almide — for `sigv4`, by the
[`sigv4`](https://github.com/O6lvl4/sigv4) library — not in the engine's
Rust.

The engine is where the request is, and it cannot call Almide: Almide calls
native code, not the other way. So nautra does here what it does for a native
command: it starts itself once more (`nautra __auth`), and the engine asks
that copy over a pipe, one line per request. The copy is started at the first
request that needs it and kept until nautra ends.

- **The copy finds the credentials, and is the only part that uses them.**
  The key is not in the request document a run is started with, and the
  engine is never handed it. (The process the engine runs in still has the
  host's environment, as it does for any secret.)
- **The copy lasts**, so a scheme that mints a token, or is handed
  credentials that expire, can keep and renew them there without the engine
  or the run knowing.

The alternatives, and why not:

- *Authenticating in the engine's Rust.* Each scheme would be written twice
  over the years — once where the policy is, once where the request is —
  and the engine would fill with providers.
- *A scheme that is a Wasm image*, run by the engine the way a tool is. It
  would let anyone add a scheme without building nautra, which is worth
  having, and the question and answer of §2 are already what such an image
  would be asked and say. It also asks how an image is granted a key and
  trusted with one, which is a design of its own.

## 5. Where the credentials come from

From the host, by the scheme's own conventions; for `sigv4`, the variables of
§3. A run that names an authenticated destination and finds the host
without them is refused before it starts.

They are the host's. The guest's environment starts empty, as always;
`--auth` does not copy anything in.

Not built: the credentials a platform hands a workload by its role, with no
key anywhere — for AWS, the container credentials endpoint and the instance
metadata service. On AgentCore Runtime that is the step that lets an agent
use its execution role. It belongs to the scheme, in the copy (§4).

## 6. The record, the audit, and what is refused

- **The audit** line of an authenticated request names the scheme:
  `"auth": "sigv4"`. It never held a header value and still does not.
- **A record** (`--record`) identifies a request by its method, its URL and
  its body's hash. Headers are not part of it, so no credential is; a replay
  sends nothing, asks for nothing and needs no credentials.
- **A native tool or check** is refused `auth`. Its requests leave through
  Porta's proxy, which puts a value on a request and does nothing else.
- **A request that cannot be authenticated** — the copy cannot be started,
  the credentials are gone, the body is too large — is refused, with the
  reason in the audit. Nothing is ever sent bare to a host that is
  authenticated to.

## Not decided yet

- **More schemes.** The ones in sight: a Google Cloud access token and an
  Azure one (each minted, or taken from the machine's metadata service), and
  an OAuth client-credentials grant. Each is a file under §2; none is built.
- **Role credentials** for `sigv4` (§5).
- **A scheme chosen by the host name**, so that `--auth HOST` alone would
  do. Possible once there is more than one scheme to choose between.
- **Bodies too large to hash.** S3 accepts a request whose body is not
  signed, which would let an upload stream.
- **Schemes as images** (§4).

None of this has been run against AWS, or any provider, yet. What is tested
is that the signature a server receives is the one the algorithm gives for
the request it received.
