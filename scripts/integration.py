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
import shutil
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

    def __call__(self, *args, env=None, cwd=None, home=None, stdin=None):
        full = {"PATH": os.environ["PATH"], "HOME": str(self.home), "NAUTRA_HOME": str(home or self.home)}
        full.update(env or {})
        done = subprocess.run([self.binary, *map(str, args)], capture_output=True, text=True, env=full, cwd=cwd, timeout=120, input=stdin)
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


def record_checks(n, project, env, asked, token):
    """--record, replay and --resume: a run written down, re-run without the world, and continued."""
    record = project / "run.record"
    made = n("run", "--record", record, "-f", "agent.toml", "What does the note say?", env=env, cwd=project)
    check("a recorded run answers as usual", made, 0, out="The note says: THE TIDE IS IN")
    text = record.read_text()
    lines = text.splitlines()
    expect("the record holds no secret", token not in text, "the key is in the record")
    check("inspect reads a record without running it", n("inspect", record), 0, out="exchanges 5 (3 http, 2 tool)")
    check("inspect says how the run ended", n("inspect", record), 0, out="finished, exit code 0")
    check("a record is not written over", n("run", "--record", record, "-f", "agent.toml", "again", env=env, cwd=project), 125, err="cannot start the record")

    before = len(asked)
    replayed = n("replay", record, cwd=project)
    check("a replay gives the same answer with no key", replayed, 0, out="The note says: THE TIDE IS IN", err="5 exchange(s) replayed, nothing sent")
    expect("a replay calls no model", len(asked) == before, f"{len(asked) - before} model call(s)")
    check("a replay runs where the record was made", n("replay", record, cwd=project.parent), 125, err="was recorded in")

    def variant(name, keep):
        path = project / name
        path.write_text("\n".join(keep) + "\n")
        return path

    entries = [json.loads(line) for line in lines]
    tool_end = next(i for i, e in enumerate(entries) if "end" in e and "content" in e["end"])
    changed = list(lines)
    changed[tool_end] = lines[tool_end].replace("the tide is in", "the tide is out")
    check("a run that leaves its record is refused", n("replay", variant("changed.record", changed), cwd=project), 3, err="left its record")
    other = dict(entries[0], wasm="sha256:" + "0" * 64)
    check("a record is tied to its binary", n("replay", variant("other.record", [json.dumps(other)] + lines[1:]), cwd=project), 125, err="not the one that was recorded")
    check("a file that is not a record is refused", n("replay", project / "agent.toml", cwd=project), 125, err="is not a nautra record")

    # Stopped after the first model call: the rest is carried out for real.
    stopped = variant("stopped.record", lines[:3])
    check("inspect sees a stopped run", n("inspect", stopped), 0, out="stopped after 1 exchange(s)")
    before = len(asked)
    resumed = n("run", "--resume", stopped, env=env, cwd=project)
    check("a stopped run is resumed to its answer", resumed, 0, out="The note says: THE TIDE IS IN", err="resumed after 1 recorded exchange(s)")
    expect("a resumed run repeats nothing it had done", len(asked) - before == 2, f"{len(asked) - before} model call(s)")
    check("a resumed run finishes its record", n("inspect", stopped), 0, out="finished, exit code 0")
    check("a finished run is not resumed", n("run", "--resume", stopped, env=env, cwd=project), 125, err="already finished")
    check("the completed record replays", n("replay", stopped, cwd=project), 0, err="5 exchange(s) replayed")

    # Stopped with a request sent and no answer written: it may have happened.
    flight = variant("flight.record", lines[:2])
    check("inspect sees an exchange in flight", n("inspect", flight), 0, out="in flight")
    before = len(asked)
    check("an exchange in flight is not repeated", n("run", "--resume", flight, env=env, cwd=project), 3, err="in flight")
    expect("nothing was sent for it", len(asked) == before, f"{len(asked) - before} model call(s)")
    check("a service cannot be recorded", n("run", "--record", project / "x.record", "-p", "8080", "-f", "agent.toml", cwd=project), 125, err="a run that ends")


def team_checks(n, wasm, project, env, port, tools):
    """Delegation, checks, and a remote MCP server reached through a tool image."""
    model = f'env = ["MODEL_URL=http://127.0.0.1:{port}/v1/chat/completions", "MODEL_NAME=fixture"]\nsecrets = ["MODEL_KEY@127.0.0.1:{port}"]\n'
    (project / "helper.toml").write_text(f'[agent]\nimage = "{wasm("agent")}"\n{model}' + tools)
    (project / "lead.toml").write_text(f'[agent]\nimage = "{wasm("agent")}"\n{model}\n[tools.helper]\nagent = "helper.toml"\ndescription = "Hand a task to the helper."\n')
    check("an agent hands work to another agent", n("run", "-f", project / "lead.toml", "delegate", "this", env=env), 0, out="the helper said: TIDE!")
    (project / "self.toml").write_text(f'[agent]\nimage = "{wasm("agent")}"\n{model}\n[tools.me]\nagent = "self.toml"\n')
    check("an agent that names itself is refused", n("run", "-f", project / "self.toml", "task", env=env), 125, err="more than 4 deep")

    # Checks: one set before a tool, one that must accept the finished work.
    ws = project / "checked"
    ws.mkdir()
    (project / "checked.toml").write_text(f"""
[agent]
image = "{wasm('toolcall')}"
audit = "checked.jsonl"

[tools.write_file]
image = "{wasm('write_file')}"
description = "Write a file."
mounts = ["checked:/work"]

[checks.only_notes]
image = "{wasm('only_notes')}"
before = ["write_file"]

[checks.has_summary]
image = "{wasm('has_summary')}"
finish = true
mounts = ["checked:/work:ro"]
""")
    call = lambda *args: n("run", "-f", project / "checked.toml", *args)
    check("a workload is offered verify when work must be accepted", call(), 4, out="verify: Run the checks")
    refused = call("write_file", '{"path": "elsewhere.txt", "text": "x"}')
    check("a check refuses a tool call, and says why", refused, 4, out="check only_notes refused it")
    check("the reason reaches the caller", refused, 4, out="under notes/ only")
    expect("a refused tool did not run", not (ws / "elsewhere.txt").exists())
    check("work that is not accepted is exit code 4", refused, 4, err="its work is not accepted: check has_summary")
    check("verify says what is missing while there is time", call("verify"), 4, out="notes/summary.txt is missing")
    done = call("write_file", '{"path": "notes/summary.txt", "text": "done"}')
    check("a permitted call runs, and accepted work ends well", done, 0, out="wrote 4 characters")
    check("verify agrees once the work is there", call("verify"), 0, out="every check passes")
    reasons = [r.get("reason", "") for r in map(json.loads, (project / "checked.jsonl").read_text().splitlines())]
    expect("a check's refusal is in the audit", any("refused by check only_notes" in r for r in reasons), repr(reasons))
    (project / "never.toml").write_text(f'[agent]\nimage = "{wasm("toolcall")}"\n[checks.c]\nimage = "{wasm("only_notes")}"\n')
    check("a check with no occasion is refused", n("run", "-f", project / "never.toml"), 125, err="would never run")

    # A remote MCP server: a tool image granted the server's host and a secret.
    remote_token = "mcp-token-s3cr3t"
    calls = []

    class Remote(http.server.BaseHTTPRequestHandler):
        def do_POST(self):
            message = json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0"))))
            calls.append((self.headers.get("Authorization"), self.headers.get("Mcp-Session-Id"), message.get("method")))
            if self.headers.get("Authorization") != f"Bearer {remote_token}":
                self.send_response(401)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            if "id" not in message:
                self.send_response(202)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            if message["method"] == "initialize":
                result, extra = {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "fixture", "version": "1"}}, [("Mcp-Session-Id", "session-1")]
            else:
                query = message["params"]["arguments"].get("query", "")
                result, extra = {"content": [{"type": "text", "text": f"found: {query}"}], "isError": False}, []
            payload = json.dumps({"jsonrpc": "2.0", "id": message["id"], "result": result})
            if self.path == "/stream" and message["method"] == "tools/call":
                body, kind = f"event: message\ndata: {payload}\n\n".encode(), "text/event-stream"
            else:
                body, kind = payload.encode(), "application/json"
            self.send_response(200)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(body)))
            for name, text in extra:
                self.send_header(name, text)
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    server = http.server.HTTPServer(("127.0.0.1", 0), Remote)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    remote = server.server_address[1]
    for path in ("mcp", "stream"):
        (project / f"remote-{path}.toml").write_text(f"""
[agent]
image = "{wasm('toolcall')}"

[tools.search]
image = "{wasm('mcp_call')}"
description = "Search."
env = ["MCP_URL=http://127.0.0.1:{remote}/{path}", "MCP_TOOL=lookup"]
secrets = ["MCP_TOKEN@127.0.0.1:{remote}"]
""")
    renv = {"MCP_TOKEN": remote_token}
    check("a remote MCP tool is called through a tool image", n("run", "-f", project / "remote-mcp.toml", "search", '{"query": "tide"}', env=renv), 0, out="found: tide", absent=remote_token)
    check("a streamed MCP reply is read", n("run", "-f", project / "remote-stream.toml", "search", '{"query": "pool"}', env=renv), 0, out="found: pool")
    expect("the remote server saw the session and the real token", ("Bearer " + remote_token, "session-1", "tools/call") in calls, repr(calls))


def native_checks(n, wasm, project):
    """Native commands as tools and checks, confined by the sandbox nautra is built with."""
    repo = project / "repo"
    repo.mkdir()
    (repo / "note.txt").write_text("the tide is in\n")
    outside = project / "outside-native.txt"

    class Hello(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"reached the listener")

        def log_message(self, *args):
            pass

    listener = http.server.HTTPServer(("127.0.0.1", 0), Hello)
    port = listener.server_address[1]
    threading.Thread(target=listener.serve_forever, daemon=True).start()
    (project / "native.toml").write_text(f"""
[agent]
image = "{wasm('toolcall')}"

[tools.count_words]
command = ["/usr/bin/wc", "-w"]
description = "Count the words given."

[tools.show]
command = ["/bin/cat", "note.txt"]
mounts = ["repo:ro"]

[tools.scribble]
command = ["/bin/sh", "-c", "echo x > new.txt"]
mounts = ["repo:ro"]

[tools.keep]
command = ["/bin/sh", "-c", "echo kept > kept.txt && cat kept.txt"]
mounts = ["repo"]

[tools.escape]
command = ["/bin/sh", "-c", "echo x > {outside}"]
mounts = ["repo"]

[tools.dial]
command = ["/usr/bin/curl", "-sS", "-m", "5", "http://127.0.0.1:{port}/"]

[tools.dial_granted]
command = ["/usr/bin/curl", "-sS", "-m", "5", "http://127.0.0.1:{port}/"]
net = ["*:{port}"]
reads = "open"

[tools.slow]
command = ["/bin/sleep", "30"]
timeout = "1s"

[tools.named]
command = ["/usr/bin/env"]
env = ["GREETING=hello", "NAUTRA_PASSED"]

[checks.note_is_there]
command = ["/bin/test", "-s", "note.txt"]
finish = true
mounts = ["repo:ro"]
""")
    call = lambda *args, **more: n("run", "-f", project / "native.toml", *args, **more)
    (project / "guest.toml").write_text(f'[agent]\nimage = "{wasm("toolcall")}"\n[tools.t]\ncommand = ["/bin/cat"]\nmounts = ["repo:/work"]\n')
    check("a native command has no guest paths", n("run", "-f", project / "guest.toml"), 125, err="sees host directories where they are")
    (project / "addr.toml").write_text(f'[agent]\nimage = "{wasm("toolcall")}"\n[tools.t]\ncommand = ["/bin/cat"]\nnet = ["10.0.0.1:443"]\n')
    check("what the sandbox cannot enforce is refused, not loosened", n("run", "-f", project / "addr.toml"), 125, err="not to an address")
    (project / "word.toml").write_text(f'[agent]\nimage = "{wasm("toolcall")}"\n[tools.t]\ncommand = ["/bin/cat"]\nporta = ["-v", "/"]\n')
    check("a sandbox word that would open something is not taken", n("run", "-f", project / "word.toml"), 125, err="not a word nautra passes")
    # No porta on PATH: the sandbox is nautra's own.
    env = {"PATH": "/usr/bin:/bin"}
    check("a native tool gets its arguments on stdin", call("count_words", '{"text": "one two"}', env=env), 0, out="2")
    check("a native tool reads its mount", call("show", env=env), 0, out="the tide is in")
    check("a native tool cannot write a read-only mount", call("scribble", env=env), 0, out="tool error")
    expect("nothing was written", not (repo / "new.txt").exists())
    check("a native tool writes its own mount", call("keep", env=env), 0, out="kept")
    check("a native tool cannot write outside its mount", call("escape", env=env), 0, out="tool error")
    expect("nothing was written outside", not outside.exists())
    check("a native tool has no network unless granted", call("dial", env=env), 0, out="tool error", absent="reached the listener")
    check("a native tool reaches the port its entry grants", call("dial_granted", env=env), 0, out="reached the listener")
    started = time.time()
    check("a native tool is stopped at its timeout", call("slow", env=env), 0, out="tool error")
    expect("the timeout came before the command would have ended", time.time() - started < 20)
    seen = call("named", env={**env, "NAUTRA_PASSED": "by-name", "NAUTRA_UNNAMED": "stays-out"})
    check("a native tool sees the variables its entry names, and no others", seen, 0, out="GREETING=hello", absent="stays-out")
    expect("a variable named without a value is copied from the host", "NAUTRA_PASSED=by-name" in seen[1], seen[1])
    listener.shutdown()
    (repo / "note.txt").write_text("")
    check("a native finish check refuses", call("show", env=env), 4, err="check note_is_there does not accept")


def compose_checks(n, wasm, work):
    """nautra.toml: a workload and the tools it may call, each an image with its own grants."""
    project = work / "project"
    (project / "ws").mkdir(parents=True)
    (project / "ws" / "note.txt").write_text("the tide is in\n")
    (project / "outside.txt").write_text("outside\n")
    tools = f"""
[tools.upper]
image = "{wasm('upper')}"
description = "Uppercase a text."
input = {{ type = "object", properties = {{ text = {{ type = "string" }} }}, required = ["text"], additionalProperties = false }}

[tools.read_file]
image = "{wasm('read_file')}"
description = "Read a file from the workspace."
input = {{ type = "object", properties = {{ path = {{ type = "string" }} }}, required = ["path"] }}
mounts = ["ws:/work:ro"]
"""
    (project / "nautra.toml").write_text(f'[agent]\nimage = "{wasm("toolcall")}"\naudit = "audit.jsonl"\n' + tools)
    call = lambda *args: n("run", "-f", project / "nautra.toml", *args)

    check("a tool runs by hand, arguments on stdin", n("run", "-i", wasm("upper"), stdin='{"text": "tide"}'), 0, out="TIDE")
    check("the workload sees its tools", call(), 0, out="read_file: Read a file from the workspace.\nupper: Uppercase a text.")
    check("a tool is called", call("upper", '{"text": "tide"}'), 0, out="TIDE")
    check("a tool uses its own mount", call("read_file", '{"path": "note.txt"}'), 0, out="the tide is in")
    check("a tool cannot leave its mount", call("read_file", '{"path": "../outside.txt"}'), 0, out="tool error", absent="outside\n")
    check("arguments are checked against the schema", call("upper", '{"txt": "x", "text": 3}'), 0, out="arguments/text: 3 is not of type")
    check("an unknown tool is refused", call("nope", "{}"), 1, err="no tool named 'nope'")
    check("without a file there is no tool endpoint", n("run", wasm("toolcall")), 1, err="denied")
    kinds = [(r["kind"], r["decision"], r.get("name")) for r in map(json.loads, (project / "audit.jsonl").read_text().splitlines())]
    expect("tool calls are recorded", ("tool", "allow", "upper") in kinds and ("tool", "deny", "nope") in kinds, repr(kinds))
    (project / "typo.toml").write_text(f'[agent]\nimage = "{wasm("toolcall")}"\n[tools.x]\nimage = "{wasm("upper")}"\nmount = []\n')
    check("a mistake in the file is refused", n("run", "-f", project / "typo.toml"), 125, err="unknown key 'mount' in [tools.x]")
    (project / "schema.toml").write_text(f'[agent]\nimage = "{wasm("toolcall")}"\n[tools.x]\nimage = "{wasm("upper")}"\ninput = {{ type = 7 }}\n')
    check("a schema that is not one stops the run", n("run", "-f", project / "schema.toml"), 125, err="input schema is not valid")

    # An agent: a model that asks for tools, and a key the agent never holds.
    token = "model-key-s3cr3t"
    asked = []

    class Model(http.server.BaseHTTPRequestHandler):
        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0"))))
            asked.append((self.headers.get("Authorization"), body))
            results = [m["content"] for m in body["messages"] if m["role"] == "tool"]
            task = body["messages"][0]["content"]
            call = lambda name, arguments: {"role": "assistant", "tool_calls": [{"id": f"c{len(results)}", "type": "function", "function": {"name": name, "arguments": json.dumps(arguments)}}]}
            if "delegate" in task:
                message = call("helper", {"task": "shout tide"}) if not results else {"role": "assistant", "content": f"the helper said: {results[0]}"}
            elif task.startswith("shout"):
                message = call("upper", {"text": task.split()[1]}) if not results else {"role": "assistant", "content": f"{results[0]}!"}
            elif "loop" in task and not any("used up" in r for r in results):
                message = {"role": "assistant", "tool_calls": [{"id": f"c{len(results)}", "type": "function", "function": {"name": "upper", "arguments": json.dumps({"text": "again"})}}]}
            elif "loop" in task:
                message = {"role": "assistant", "content": f"stopped after {len(results) - 1} calls"}
            elif not results:
                message = {"role": "assistant", "tool_calls": [{"id": "c0", "type": "function", "function": {"name": "read_file", "arguments": json.dumps({"path": "note.txt"})}}]}
            elif len(results) == 1:
                message = {"role": "assistant", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "upper", "arguments": json.dumps({"text": results[0]})}}]}
            else:
                message = {"role": "assistant", "content": f"The note says: {results[1]}"}
            reply = json.dumps({"choices": [{"message": message}]}).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(reply)))
            self.end_headers()
            self.wfile.write(reply)

        def log_message(self, *args):
            pass

    server = http.server.HTTPServer(("127.0.0.1", 0), Model)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    port = server.server_address[1]
    (project / "agent.toml").write_text(
        f'[agent]\nimage = "{wasm("agent")}"\nsecrets = ["MODEL_KEY@127.0.0.1:{port}"]\n'
        f'env = ["MODEL_URL=http://127.0.0.1:{port}/v1/chat/completions", "MODEL_NAME=fixture"]\nmax_tool_calls = 3\ntimeout = "30s"\n' + tools)
    env = {"MODEL_KEY": token}
    done = n("run", "-f", project / "agent.toml", "What", "does", "the", "note", "say?", env=env)
    check("an agent uses its tools to answer", done, 0, out="The note says: THE TIDE IS IN", absent=token)
    expect("the model was offered the tools", [t["function"]["name"] for t in asked[0][1]["tools"]] == ["read_file", "upper"], repr(asked[0][1].get("tools")))
    expect("the model received the real key", all(auth == f"Bearer {token}" for auth, _ in asked), repr([a for a, _ in asked]))
    looped = n("run", "-f", project / "agent.toml", "loop", env=env)
    check("the run's tool calls have a ceiling", looped, 0, out="stopped after 3 calls")
    check("an agent needs its key", n("run", "-f", project / "agent.toml", "task"), 125, err="MODEL_KEY is not set")
    record_checks(n, project, env, asked, token)
    team_checks(n, wasm, project, env, port, tools)
    native_checks(n, wasm, project)

    started = n("run", "-d", "--name", "agent-bg", "-f", project / "agent.toml", "What does the note say?", env=env)
    check("a composition runs in the background", started, 0)
    for _ in range(100):
        if "exited" in n("ps", "-a")[1]:
            break
        time.sleep(0.1)
    check("its answer is in its log", n("logs", "agent-bg"), 0, out="The note says: THE TIDE IS IN")
    n("rm", "agent-bg")


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
    compose_checks(n, wasm, work)
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
