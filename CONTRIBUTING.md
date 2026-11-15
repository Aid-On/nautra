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

- Contributions are accepted under the [Contributor License Agreement](CLA.md).
  Opening a pull request means you agree to it.
- Code outside `ee/` is Apache-2.0; code in `ee/` is under the
  [Nautra Enterprise License](ee/LICENSE). See [LICENSING.md](LICENSING.md).
- Start every new source file with the SPDX header for its directory:
  `SPDX-License-Identifier: Apache-2.0` outside `ee/`,
  `SPDX-License-Identifier: LicenseRef-Nautra-Enterprise` inside it.
- Core must not depend on `ee/`.

## Security

Do not report vulnerabilities in public issues — see [SECURITY.md](SECURITY.md).
