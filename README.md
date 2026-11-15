<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./docs/assets/brand/nautra-cover-dark.png">
    <img src="./docs/assets/brand/nautra-cover.png" alt="Nautra" width="720">
  </picture>
</p>

<p align="center"><strong>Build, ship and run WebAssembly workloads.</strong></p>

<p align="center">
  <a href="./LICENSING.md"><img src="https://img.shields.io/badge/license-Apache--2.0%20%2B%20Enterprise-6447f0.svg" alt="License: Apache-2.0 + Nautra Enterprise"></a>
  <img src="https://img.shields.io/badge/status-pre--alpha-c679fc.svg" alt="Status: pre-alpha">
</p>

<p align="center">
  <a href="#what-it-is-meant-to-be">What</a> ·
  <a href="#project-status">Status</a> ·
  <a href="#editions-and-licence">Editions</a> ·
  <a href="./LICENSING.md">Licensing</a> ·
  <a href="./CONTRIBUTING.md">Contributing</a> ·
  <a href="./SECURITY.md">Security</a>
</p>

<p align="center">
  <a href="README.md"><img alt="English" src="https://img.shields.io/badge/English-24292f?style=flat-square"></a>
  <a href="README.ja.md"><img alt="日本語" src="https://img.shields.io/badge/%E6%97%A5%E6%9C%AC%E8%AA%9E-d0d7de?style=flat-square"></a>
</p>

## What it is meant to be

Nautra is to Wasm components what a container engine is to Linux containers.

A Wasm module is already a sandbox: it starts with no filesystem, no network
and no ambient authority, and gets only the capabilities it is handed. Nautra
is the tooling around that unit — packaging it as an image, moving it through
registries, and running it with explicitly granted capabilities — so that a
Wasm workload can be handled with the same ease as a container, without a
guest kernel or a daemon.

Nautra is developed by [Aid-On](https://github.com/Aid-On). Its sibling,
[Porta](https://github.com/almide/porta), confines native processes with the
host kernel; Nautra's unit is the Wasm component.

## Project status

**Not yet usable.** This repository currently holds the project's name, brand,
licence structure and contribution terms. There is no code and no release.

## Editions and licence

Nautra is open core:

| | Community Edition | Nautra Enterprise |
|---|---|---|
| Source | everything outside [`ee/`](ee/) | adds the contents of [`ee/`](ee/) |
| Licence | [Apache License 2.0](LICENSE) | [Nautra Enterprise License](ee/LICENSE), with a subscription from Aid-On |

The split, and the rules that keep it clean, are in
[LICENSING.md](LICENSING.md). The name and logo are covered by
[TRADEMARKS.md](TRADEMARKS.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Contributions are accepted under the
[Contributor License Agreement](CLA.md). Report vulnerabilities privately, as
described in [SECURITY.md](SECURITY.md).

## The name

*Nautra* is a coined word built on *naut-*, from *nautilus* — the animal in
the logo, which lives in the outermost chamber of a shell it seals off behind
itself.
