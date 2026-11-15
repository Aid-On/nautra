# Licensing

Nautra is open core. One repository, two licenses, split by directory.

| Path | License | SPDX identifier | What it means |
|---|---|---|---|
| everything outside `ee/` | [Apache License 2.0](LICENSE) | `Apache-2.0` | Use, modify and redistribute freely, including commercially. |
| `ee/` | [Nautra Enterprise License](ee/LICENSE) | `LicenseRef-Nautra-Enterprise` | Source-available. Production use needs a Nautra Enterprise subscription from Aid-On. |

## Rules that keep the split clean

1. **The directory decides.** A file's license is determined by where it lives.
   Every source file also carries an `SPDX-License-Identifier` header that
   matches its directory.
2. **Core never depends on `ee/`.** Code outside `ee/` must build, test and run
   with the `ee/` directory deleted. `ee/` may depend on core, never the
   reverse. A build without `ee/` is the Nautra Community Edition and is
   entirely Apache-2.0.
3. **Nothing moves from core into `ee/`.** What has been released under
   Apache-2.0 stays available under Apache-2.0. Features may move from `ee/`
   to core, not the other way.
4. **Third-party code keeps its own license** and is listed in
   `THIRD_PARTY_LICENSES.md` once dependencies exist.

## Editions

| | Community Edition | Nautra Enterprise |
|---|---|---|
| Source | this repository without `ee/` | this repository including `ee/` |
| License | Apache-2.0 | Nautra Enterprise License + subscription |
| Provided by | anyone | Aid-On |

What belongs in which edition is decided by who needs it: capabilities an
individual developer or a single team needs to build, ship and run Wasm
workloads belong in core. Capabilities that exist because an organisation
operates Nautra at scale — organisation-wide policy, audit, identity
integration, fleet management, support — belong in `ee/`.

## Name and logo

The Apache License does not grant trademark rights (section 6). The name
"Nautra", the name "Nautra Enterprise" and the logo are governed by
[TRADEMARKS.md](TRADEMARKS.md).

## Contributions

Contributions are accepted under the [Contributor License Agreement](CLA.md).
See [CONTRIBUTING.md](CONTRIBUTING.md).
