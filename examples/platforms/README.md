# On hosted platforms

What goes in a container for a platform that hosts agents. The reasoning is
in [ADR-0003](../../docs/adr/0003-platforms.md).

None of these has been deployed yet. What is tested is nautra's side: the
integration suite checks that a file with `[invoke]` or `[mcp]` answers the
requests each platform's documentation says it sends, and
`scripts/check-example-images.sh` builds these images and asks the running
containers the same.

| | image | file |
|---|---|---|
| [agentcore](agentcore/) | linux/arm64 | an agent behind `[invoke]` on 8080 |
| [cloudflare](cloudflare/) | linux/amd64 | tools behind `[mcp]` on 8000 |

Both start from `ghcr.io/aid-on/nautra`, which is published for both
architectures, and add a composition file and the `.wasm` files it names.
Build the example guests first: `bash scripts/build-examples.sh`.
