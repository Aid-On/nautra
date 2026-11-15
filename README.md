# Nautra

Build, ship and run WebAssembly workloads — what a container engine is for
Linux containers, for Wasm components.

> **Status: not yet usable.** This repository currently holds the project's
> name, licence structure and contribution terms. There is no code and no
> release.

[日本語](README.ja.md)

## What it is meant to be

A Wasm module is already a sandbox: it starts with no filesystem, no network
and no ambient authority, and gets only the capabilities it is handed. Nautra
is the tooling around that unit — packaging it as an image, moving it through
registries, and running it with explicitly granted capabilities — so that a
Wasm workload can be handled with the same ease as a container, without a
guest kernel or a daemon.

Nautra is developed by [Aid-On](https://github.com/Aid-On). Its sibling,
[Porta](https://github.com/almide/porta), confines native processes with the
host kernel; Nautra's unit is the Wasm component.

## Editions and licence

Nautra is open core:

- **Community Edition** — everything outside [`ee/`](ee/), under the
  [Apache License 2.0](LICENSE).
- **Nautra Enterprise** — adds the contents of [`ee/`](ee/), under the
  [Nautra Enterprise License](ee/LICENSE), with a subscription from Aid-On.

Details: [LICENSING.md](LICENSING.md) · name and logo:
[TRADEMARKS.md](TRADEMARKS.md) · contributing: [CONTRIBUTING.md](CONTRIBUTING.md) ·
security: [SECURITY.md](SECURITY.md)

## The name

*Nautra* (ノートラ) is a coined word built on *naut-*, from *nautilus*.
