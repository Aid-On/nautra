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
import json
import os
import pathlib
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request

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
    class Quiet(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            body = b"hello from the host\n"
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    server = http.server.HTTPServer(("127.0.0.1", 0), Quiet)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server.server_address[1]


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def fetch(url, data=None):
    """(status, body) of one request, or (0, reason) when it could not be made."""
    try:
        with urllib.request.urlopen(urllib.request.Request(url, data=data), timeout=10) as reply:
            return reply.status, reply.read().decode()
    except urllib.error.HTTPError as error:
        return error.code, ""
    except OSError as error:
        return 0, str(error)


def expect(name, condition, detail=""):
    if condition:
        print(f"ok    {name}")
    else:
        failures.append(name)
        print(f"FAIL  {name}: {detail}")


def service_checks(n, wasm):
    """An HTTP component published with -p, requested, and stopped."""
    port = free_port()
    env = {"PATH": os.environ["PATH"], "HOME": str(n.home), "NAUTRA_HOME": str(n.home)}
    server = subprocess.Popen([n.binary, "run", "-p", str(port), "--timeout", "5s", str(wasm("web"))], env=env, stderr=subprocess.PIPE, text=True)
    try:
        for _ in range(100):
            if fetch(f"http://127.0.0.1:{port}/ready")[0]:
                break
            time.sleep(0.1)
        status, body = fetch(f"http://127.0.0.1:{port}/tide/pool")
        expect("a published service answers", status == 200 and "GET /tide/pool" in body, f"{status} {body!r}")
        status, body = fetch(f"http://127.0.0.1:{port}/in", data=b"x")
        expect("a published service sees the method", status == 200 and "POST /in" in body, f"{status} {body!r}")
        replies = []
        threads = [threading.Thread(target=lambda i=i: replies.append(fetch(f"http://127.0.0.1:{port}/c{i}"))) for i in range(16)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        expect("requests are served side by side", sorted(body for _, body in replies) == sorted(f"GET /c{i} from a wasm component\n" for i in range(16)), repr(replies))
    finally:
        server.send_signal(signal.SIGTERM)
        try:
            code = server.wait(timeout=10)
        except subprocess.TimeoutExpired:
            server.kill()
            code = None
        expect("a service stops cleanly on SIGTERM", code == 0, f"exit {code}")
        expect("a service says where it listens", f"serving on http://127.0.0.1:{port}" in server.stderr.read())


def secret_checks(n, wasm, work):
    """--secret: the guest holds a placeholder; the value reaches only the bound host."""
    seen = []

    class Recorder(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            seen.append(self.headers.get("Authorization"))
            self.send_response(200)
            self.send_header("Content-Length", "3")
            self.end_headers()
            self.wfile.write(b"ok\n")

        def log_message(self, *args):
            pass

    def recorder():
        server = http.server.HTTPServer(("127.0.0.1", 0), Recorder)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        return server.server_address[1]

    bound, other = recorder(), recorder()
    token = "real-token-s3cr3t"
    audit = work / "audit.jsonl"
    env = {"API_TOKEN": token}
    got = n("run", "--secret", f"API_TOKEN@127.0.0.1:{bound}", "--audit", audit, wasm("fetch"), f"http://127.0.0.1:{bound}/v1/models?key=q", env=env)
    check("a secret's host is reachable over http", got, 0, out="200 3 characters")
    check("the guest holds a placeholder", got, 0, out="nautra-secret-", absent=token)
    expect("the bound host receives the real value", seen == [f"Bearer {token}"], repr(seen))
    stolen = n("run", "--secret", f"API_TOKEN@127.0.0.1:{bound}", "--allow-net", f"127.0.0.1:{other}", "--audit", audit, wasm("fetch"), f"http://127.0.0.1:{other}/steal", env=env)
    check("a secret is not sent to another host", stolen, 1, err="denied", absent=token)
    expect("the other host received nothing", len(seen) == 1, repr(seen))
    check("the guest's environment has no real value", n("run", "--secret", "API_TOKEN@api.example.com", wasm("env"), env=env), 0, out="API_TOKEN=nautra-secret-", absent=token)
    check("a secret's host is not opened to sockets", n("run", "--secret", f"API_TOKEN@127.0.0.1:{bound}", wasm("dial"), "127.0.0.1", bound, env=env), 1, err="permission denied")
    check("a secret missing from the host is refused", n("run", "--secret", "API_TOKEN@api.example.com", wasm("env")), 125, err="is not set")
    check("a secret needs a host", n("run", "--secret", "API_TOKEN", wasm("env"), env=env), 125, err="binds a secret to no host")

    n("run", "--allow-net", f"127.0.0.1:{bound}", "--audit", audit, wasm("dial"), "127.0.0.1", bound)
    text = audit.read_text()
    records = [json.loads(line) for line in text.splitlines()]
    allowed = [r for r in records if r["kind"] == "http" and r["decision"] == "allow"]
    denied = [r for r in records if r["kind"] == "http" and r["decision"] == "deny"]
    expect("an allowed request is recorded with its status and secret", len(allowed) == 1 and allowed[0]["status"] == 200 and allowed[0]["secrets"] == ["API_TOKEN"] and allowed[0]["path"] == "/v1/models", repr(allowed))
    expect("a refused request is recorded with its reason", len(denied) == 1 and "not bound" in denied[0]["reason"], repr(denied))
    expect("a socket connection is recorded", any(r["kind"] == "socket" and r["decision"] == "allow" and r["port"] == bound for r in records), text)
    expect("the record holds no secret and no query string", token not in text and "key=q" not in text, text)


def background_checks(n, wasm):
    """run -d, ps, logs, stop and rm: a run that outlives the command that started it."""
    port = free_port()
    started = n("run", "-d", "--name", "web", "-p", port, wasm("web"))
    check("run -d prints an id", started, 0)
    run_id = started[1].strip()
    expect("the id is twelve hex digits", len(run_id) == 12 and all(c in "0123456789abcdef" for c in run_id), repr(run_id))
    for _ in range(50):
        if fetch(f"http://127.0.0.1:{port}/")[0]:
            break
        time.sleep(0.1)
    expect("a background service answers", "GET /bg" in fetch(f"http://127.0.0.1:{port}/bg")[1])
    check("ps lists a running run", n("ps"), 0, out="running")
    check("ps shows where it is published", n("ps"), 0, out=f"127.0.0.1:{port}")
    check("logs shows what it wrote", n("logs", "web"), 0, out=f"serving on http://127.0.0.1:{port}")
    check("a name is used once", n("run", "-d", "--name", "web", "-p", free_port(), wasm("web")), 125, err="already exists")
    check("a run that fails at once says so", n("run", "-d", "--name", "clash", "-p", port, wasm("web")), 125, err="ended at once")
    check("a running run is not removed", n("rm", "web"), 125, err="is running")
    check("a run is found by the start of its id", n("stop", run_id[:6]), 0, out="web")
    expect("a stopped service no longer answers", fetch(f"http://127.0.0.1:{port}/")[0] == 0)
    check("ps hides what has ended", n("ps"), 0, absent="web")
    check("ps -a shows how it ended", n("ps", "-a"), 0, out="exited (0)")
    check("stop of an ended run fails", n("stop", "web"), 1, err="is not running")

    check("a background command runs to its end", n("run", "-d", "--name", "hi", wasm("hello"), "tide"), 0)
    for _ in range(50):
        if "exited" in n("ps", "-a")[1].split("hi")[-1].splitlines()[0]:
            break
        time.sleep(0.1)
    check("its output is kept", n("logs", "hi"), 0, out="hello, tide")
    check("logs -f ends when the run has ended", n("logs", "-f", "hi"), 0, out="hello, tide")

    check("a looping run starts", n("run", "-d", "--name", "loop", wasm("spin")), 0)
    check("it is running", n("ps"), 0, out="loop")
    check("rm -f stops and forgets it", n("rm", "-f", "loop"), 0, out="loop")
    check("rm forgets ended runs", n("rm", "web", "hi", "clash"), 0, out="clash")
    check("nothing is left", n("ps", "-a"), 0, absent="exited")
    expect("no run directory is left", not any((n.home / "runs").iterdir()))

    check("--rm leaves no record", n("run", "-d", "--rm", "--name", "brief", wasm("hello")), 0)
    for _ in range(50):
        if not any((n.home / "runs").iterdir()):
            break
        time.sleep(0.1)
    expect("the --rm run is forgotten when it ends", not any((n.home / "runs").iterdir()))
    check("-d has no terminal for -i", n("run", "-d", "-i", wasm("hello")), 125, err="without a terminal")
    check("an unknown run is refused", n("logs", "absent"), 125, err="no run named")


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

    # HTTP out: `wasi:http` requests are held to the same grants, by host name.
    get = lambda url, *grants: n("run", *grants, wasm("fetch"), url)
    check("no outbound http by default", get(f"http://127.0.0.1:{port}/"), 1, err="denied")
    check("a granted host is fetched", get(f"http://127.0.0.1:{port}/", "--allow-net", f"127.0.0.1:{port}"), 0, out="characters")
    check("outbound http on another port is refused", get(f"http://127.0.0.1:{port}/", "--allow-net", "127.0.0.1:9"), 1, err="denied")
    check("outbound http is granted by the name in the URL", get(f"http://127.0.0.1:{port}/", "--allow-net", f"localhost:{port}"), 1, err="denied")

    # HTTP in: a service is published where -p says, and nowhere otherwise.
    check("a service needs -p", n("run", wasm("web")), 125, err="publish it with -p")
    check("-p needs a service", n("run", "-p", free_port(), wasm("hello")), 125, err="no HTTP handler")
    check("-p takes one port, not a mapping", n("run", "-p", "8080:80", wasm("web")), 125, err="no port of its own")
    service_checks(n, wasm)
    secret_checks(n, wasm, work)
    background_checks(n, wasm)

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
