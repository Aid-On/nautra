# ee/ — Nautra Enterprise

Everything in this directory is licensed under the
[Nautra Enterprise License](LICENSE), not Apache-2.0. Production use needs a
Nautra Enterprise subscription from Aid-On.

- Core (outside `ee/`) must never import from here; it has to build and pass
  its tests with this directory deleted.
- Every source file here starts with
  `SPDX-License-Identifier: LicenseRef-Nautra-Enterprise`.

See [LICENSING.md](../LICENSING.md) for what belongs here and what belongs in
core.
