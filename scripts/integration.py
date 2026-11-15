#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""End-to-end checks of a built nautra binary against the example guests.

    bash scripts/build-examples.sh
    python3 scripts/integration.py target/nautra

Every run uses a throwaway NAUTRA_HOME. Set NAUTRA_TEST_REGISTRY to the
address of a scratch OCI registry on this machine (for example
`localhost:5057`, from `docker run -d -p 127.0.0.1:5057:5000 registry:2`) to
also check push and pull.
"""
import http.server
import os
import pathlib
import subprocess
import sys
import tempfile
import threading

ROOT = pathlib.Path(__file__).resolve().parent.parent
WASM = ROOT / "target" / "wasm"
failures = []


class Nautra:
    def __init__(self, binary, home):
        self.binary = str(pathlib.Path(binary).resolve())
        self.home = home

    def __call__(self, *args, env=None, cwd=None, home=None):
        full = {"PATH": os.environ["PATH"], "HOME": str(self.home), "NAUTRA_HOME": str(home or self.home)}
        full.update(env or {})
        done = subprocess.run([self.binary, *map(str, args)], capture_output=True, text=True, env=full, cwd=cwd, timeout=120)
        return done.returncode, done.stdout, done.stderr


def check(name, got, want_code, out=None, err=None, absent=None):
    code, stdout, stderr = got
    problems = []
    if code != want_code:
        problems.append(f"exit {code}, expected {want_code}")
    if out is not None and out not in stdout:
        problems.append(f"stdout lacks {out!r}")
    if err is not None and err not in stderr:
        problems.append(f"stderr lacks {err!r}")
    if absent is not None and absent in stdout + stderr:
        problems.append(f"output has {absent!r}")
    if problems:
        failures.append(name)
        print(f"FAIL  {name}: {'; '.join(problems)}\n      stdout: {stdout.strip()!r}\n      stderr: {stderr.strip()!r}")
    else:
        print(f"ok    {name}")


def listener():
    """A TCP port on loopback that accepts connections, for the network checks."""
    server = http.server.HTTPServer(("127.0.0.1", 0), http.server.BaseHTTPRequestHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server.server_address[1]


def run_checks(n, work):
    wasm = lambda name: WASM / f"{name}.wasm"

    check("version", n("version"), 0, out="nautra ")
    check("help lists run", n("help"), 0, out="run [grants]")
    check("unknown command is refused", n("frob"), 125, err="unknown command")

    # A file runs as it is, with the guest's own arguments and exit code.
    check("run a module file", n("run", wasm("hello"), "tide"), 0, out="hello, tide")
    check("run a WASI 0.2 component", n("run", WASM / "motd.component.wasm"), 0, out="the tide is in")
    check("unknown option is refused", n("run", "--privileged", wasm("hello")), 125, err="unknown option")
    check("missing image is refused", n("run", "nope"), 125, err="no image named nope:latest")

    # Environment: empty unless granted; the host's variables do not leak.
    check("environment starts empty", n("run", wasm("env"), env={"HOST_SECRET": "x"}), 0, out="(no environment)", absent="HOST_SECRET")
    check("environment is granted by name", n("run", "-e", "A=1", "-e", "TOKEN", "-e", "ABSENT", wasm("env"), env={"TOKEN": "abc"}), 0, out="A=1\nTOKEN=abc")

    # Files: nothing without a mount, reads only under :ro, no way out of it.
    data = work / "data"
    data.mkdir()
    (data / "in.txt").write_text("tide\n")
    (work / "outside.txt").write_text("outside\n")
    check("no mount, no file", n("run", wasm("files")), 1, err="/data/in.txt")
    check("read-only mount refuses writes", n("run", "-v", f"{data}:/data:ro", wasm("files")), 1, err="/data/out.txt")
    check("mount allows read and write", n("run", "-v", f"{data}:/data", wasm("files")), 0, out="copied 5 characters")
    if (data / "out.txt").read_text() != "TIDE\n":
        failures.append("mounted write reached the host")
        print("FAIL  mounted write reached the host")
    check("a mount cannot be climbed out of", n("run", "-v", f"{data}:/data", wasm("files"), "/data/../outside.txt", "/data/x"), 1, err="outside.txt")
    (data / "link.txt").symlink_to(work / "outside.txt")
    check("a symlink does not lead out of a mount", n("run", "-v", f"{data}:/data", wasm("files"), "/data/link.txt", "/data/y"), 1, err="link.txt")
    check("a mount must be a directory", n("run", "-v", f"{work}/absent:/data", wasm("files")), 125, err="not a directory")

    # Ceilings.
    check("deadline stops a loop", n("run", "--timeout", "300ms", wasm("spin")), 124, err="deadline")
    check("fuel stops a loop", n("run", "--fuel", "1000000", wasm("spin")), 137, err="fuel")
    check("memory ceiling stops growth", n("run", "-m", "8m", wasm("grow")), 1, err="out of memory")

    # Network: closed unless the destination is granted.
    port = listener()
    dial = lambda *grants: n("run", *grants, wasm("dial"), "127.0.0.1", port)
    check("no network by default", dial(), 1, err="permission denied")
    check("granted destination connects", dial("--allow-net", f"127.0.0.1:{port}"), 0, out="connected")
    check("another port stays closed", dial("--allow-net", "127.0.0.1:9"), 1, err="permission denied")
    check("another host stays closed", dial("--allow-net", f"192.0.2.1:{port}"), 1, err="permission denied")
    check("a name is granted by what it resolves to", n("run", "--allow-net", f"localhost:{port}", wasm("dial"), "localhost", port), 0, out="connected")
    check("* opens every destination", dial("--allow-net", "*"), 0, out="connected")

    # Images: build, name, inspect, remove.
    (work / "files.wasm").write_bytes(wasm("files").read_bytes())
    (work / "Nautrafile").write_text(
        'wasm = "files.wasm"\ndescription = "Uppercases a file"\n\n'
        '[run]\nargs = ["/data/in.txt", "/data/out.txt"]\n\n[run.env]\nLOG = "info"\n\n'
        '[wants]\nmounts = ["/data"]\nnet = ["api.example.com:443"]\nenv = ["API_TOKEN"]\n'
    )
    pinned = {"SOURCE_DATE_EPOCH": "1700000000"}
    first = n("build", "-t", "upper:v1", work, env=pinned)
    check("build from a Nautrafile", first, 0, out="upper:v1  sha256:")
    again = n("build", "-t", "upper:again", work, env=pinned)
    if first[1].split()[-1:] != again[1].split()[-1:]:
        failures.append("pinned builds are reproducible")
        print("FAIL  pinned builds are reproducible")
    else:
        print("ok    pinned builds are reproducible")
    check("images lists it", n("images"), 0, out="upper:v1")
    check("inspect shows what it asks for", n("inspect", "upper:v1"), 0, out="asks for  -v <dir>:/data, --allow-net api.example.com:443, -e API_TOKEN")
    check("an image's wants are named, not granted", n("run", "upper:v1"), 1, err="asks for grants this run does not give: -v <dir>:/data")
    check("an image runs with its own arguments", n("run", "-v", f"{data}:/data", "upper:v1"), 0, out="copied 5 characters from /data/in.txt")
    check("build straight from a .wasm file", n("build", "-t", "hello:v1", wasm("hello")), 0, out="hello:v1  sha256:")
    check("tag adds a name", n("tag", "hello:v1", "hello:latest"), 0, out="hello:latest")
    check("rmi removes a name", n("rmi", "hello:v1"), 0, out="removed hello:v1")
    check("the other name still runs", n("run", "hello", "again"), 0, out="hello, again")
    check("rmi of nothing fails", n("rmi", "hello:v1"), 1, err="no image named hello:v1")
    (work / "Typo").write_text('wasm = "files.wasm"\n[want]\nmounts = []\n')
    check("a Nautrafile typo is an error", n("build", "-f", work / "Typo"), 125, err="unknown key 'want'")

    # A blob that no longer matches its digest is not run.
    blobs = n.home / "store" / "blobs" / "sha256"
    largest = max(blobs.iterdir(), key=lambda p: p.stat().st_size)
    original = largest.read_bytes()
    largest.write_bytes(original[:-1] + bytes([original[-1] ^ 1]))
    check("a tampered blob is refused", n("run", "-v", f"{data}:/data", "upper:v1"), 125, err="corrupt")
    largest.write_bytes(original)

    check("a local name cannot be pushed", n("push", "upper:v1"), 125, err="names no registry")


def registry_checks(n, registry, work):
    name = f"{registry}/nautra-test/hello:v1"
    check("build for a registry", n("build", "-t", name, WASM / "hello.wasm"), 0, out=name)
    check("push", n("push", name), 0, out=name)
    check("push again uploads nothing", n("push", name), 0, out="(0 blobs uploaded)")
    elsewhere = work / "elsewhere"
    check("run pulls what is not stored", n("run", name, "registry", home=elsewhere), 0, out="hello, registry", err="pulling")
    check("pull lists it", n("images", home=elsewhere), 0, out=name)
    check("a missing image is not found", n("pull", f"{registry}/nautra-test/absent:v1", home=elsewhere), 125, err="not found")


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    if not (WASM / "hello.wasm").exists():
        sys.exit("no example guests: run `bash scripts/build-examples.sh` first")
    with tempfile.TemporaryDirectory(prefix="nautra-it-") as scratch:
        scratch = pathlib.Path(scratch).resolve()
        work = scratch / "work"
        work.mkdir()
        n = Nautra(sys.argv[1], scratch / "home")
        run_checks(n, work)
        registry = os.environ.get("NAUTRA_TEST_REGISTRY")
        if registry:
            registry_checks(n, registry, work)
        else:
            print("skip  push and pull (NAUTRA_TEST_REGISTRY is not set)")
    if failures:
        sys.exit(f"\n{len(failures)} check(s) failed")
    print("\nall checks passed")


if __name__ == "__main__":
    main()
