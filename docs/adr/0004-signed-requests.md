# ADR-0004: Requests the host signs

Status: proposed, 2026-11-16

A secret (ADR-0002) is a value: nautra puts it on a request where the guest
wrote its placeholder. Some services take no such value. AWS wants every
request signed (Signature Version 4): a keyed hash over the method, the path,
the query, some headers, the body and the time, made with a key that is never
sent. There is nothing for a placeholder to stand for, so ADR-0003 left
"signed model requests" open. This decides it, for AWS first — it is what
running an agent against Amazon Bedrock needs.

## 1. A destination is signed for, not a value handed over

```toml
[agent]
image = "agent.wasm"
sign = ["aws:us-east-1:bedrock@bedrock-runtime.us-east-1.amazonaws.com"]
```

On the command line, `--sign aws:REGION:SERVICE@HOST[:PORT][,HOST…]`.

- **The guest holds nothing**, not even a placeholder. It sends the request
  it would send to an open endpoint; nautra signs it on the way out. A guest
  cannot ask for a signature, see one, or choose what is signed.
- **Every request to the host is signed**, and only those. Naming the host is
  also the grant to send HTTP there, as for a secret, and for the same
  reason.
- **Over HTTPS only**, except to this machine's loopback.
- **Region and service are written, not guessed.** The name a service signs
  under is not always the first label of its host (`bedrock-runtime` signs
  as `bedrock`), and a table of exceptions would be wrong the day a service
  is added.
- **`aws` is the only scheme.** The word is there so another can be added
  without a new key.

The same key works in a tool's table, with the tool's own destinations. A
native command cannot be signed for (§5).

## 2. What is signed

The request as it leaves: method, path and query as the guest wrote them, the
whole body, and these headers:

- `host` and `x-amz-date`, always;
- `x-amz-security-token`, when the credentials are temporary;
- `content-type`, and every `x-amz-…` header the guest set (`x-amz-target`
  is how some services name the operation).

Other headers travel unsigned, so a proxy that rewrites one does not break
the signature.

`authorization`, `x-amz-date`, `x-amz-security-token` and
`x-amz-content-sha256` are nautra's to write. A guest that sends its own has
them replaced: it cannot bring a signature made elsewhere through a host
nautra signs for.

The body is read whole before the request is sent, because its hash is part
of the signature. A body over 32 MiB is refused. A request that streams its
body therefore leaves at once rather than as it is written; the response
streams as before.

## 3. Where the signing happens

In nautra, in Almide, by the [`sigv4`](https://github.com/O6lvl4/sigv4)
library — not in the engine's Rust.

The engine is where the request is, and it cannot call Almide: Almide calls
native code, not the other way. So nautra does here what it does for a native
command: it starts itself once more (`nautra __sign`), and the engine asks
that copy over a pipe, one line per request. The copy is started at the first
request that needs a signature and kept until nautra ends.

- **The copy finds the credentials, and is the only part that uses them.**
  The engine sends it the request's shape (method, host, path, query, the
  headers to sign, the body's hash, region and service) and gets headers
  back. The secret key is not in the request document a run is started
  with, and the engine is never handed it. (The process the engine runs in
  still has the host's environment, as it does for any secret.)
- **The copy is a function.** It reads no file, opens no connection, and
  gives the same answer for the same question and the same second.

The alternatives, and why not:

- *Signing in the engine's Rust.* A hundred lines, and a second
  implementation of something nautra's own language now has a tested library
  for. Where credentials come from (§4) would be written in Rust too.
- *A signer that is a Wasm image*, run by the engine the way a tool is.
  It would let anyone add a scheme, which is worth having. It also asks how
  such an image is granted a key and trusted with one, which is a design of
  its own; nothing here is in its way.

## 4. Where the credentials come from

From the host's environment, by the names AWS uses: `AWS_ACCESS_KEY_ID`,
`AWS_SECRET_ACCESS_KEY`, and `AWS_SESSION_TOKEN` when the key is temporary.
A run that names a signed destination and finds no key is refused before it
starts.

They are the host's variables. The guest's environment starts empty, as
always; `sign` does not copy them in.

Not built: the credentials a platform hands a container by its role — the
container credentials endpoint (`AWS_CONTAINER_CREDENTIALS_FULL_URI`) and
the instance metadata service. They expire and are renewed, which is why the
copy that signs is the one that finds them: it can fetch and renew without
the engine or the run knowing. On AgentCore Runtime this is the step that
lets an agent use its execution role with no key in the image or its
configuration.

## 5. The record, the audit, and what is refused

- **The audit** line of a signed request carries `"signed": "aws"`. It never
  held a header value and still does not.
- **A record** (`--record`) identifies a request by its method, its URL and
  its body's hash. Headers are not part of it, so a signature is not; a
  replay sends nothing and needs no credentials.
- **A native tool or check** is refused `sign`. Its requests leave through
  Porta's proxy, which puts a value on a request and does not sign one.
- **A signature that cannot be made** — the copy cannot be started, the
  credentials are gone, the body is too large — refuses the request, with
  the reason in the audit. Nothing is ever sent unsigned to a host that is
  signed for.

## Not decided yet

- **Role credentials** (§4).
- **Other schemes.** Google Cloud and Azure authenticate with a bearer token
  that is minted from a key and expires. That is a secret whose value is
  fetched, not a signature, and belongs with secrets.
- **S3-sized bodies.** S3 accepts a request whose body is not signed
  (`UNSIGNED-PAYLOAD`), which would let an upload stream.
- **Signers as images** (§3).

None of this has been run against AWS yet. What is tested is that the
signature a server receives is the one AWS's algorithm gives for the request
it received.
