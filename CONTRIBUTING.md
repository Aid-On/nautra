# Contributing

Nautra is at the very start; the design is still being written down. Open an
issue before sending anything larger than a small fix.

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
