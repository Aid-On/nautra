# Contributing

Nautra is pre-alpha. Open an issue before sending anything larger than a small
fix, and read [docs/adr/0001-foundations.md](docs/adr/0001-foundations.md)
first: it records the decisions a change has to fit.

## Build and test

You need Rust 1.97 or newer.

```bash
bash scripts/install-almide.sh
.tools/almide/almide test
.tools/almide/almide build src/main.almd -o target/nautra
bash scripts/build-examples.sh
python3 scripts/integration.py target/nautra
```

A change in behaviour comes with a check in `scripts/integration.py` or a
`test` block beside the code.

## Licence of contributions

- Nautra is [Apache-2.0](LICENSE), and a contribution is accepted under the
  same licence (section 5 of it). There is nothing to sign.
- Start every new source file with `SPDX-License-Identifier: Apache-2.0`.

## Security

Do not report vulnerabilities in public issues — see [SECURITY.md](SECURITY.md).
