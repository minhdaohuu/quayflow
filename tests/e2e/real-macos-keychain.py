#!/usr/bin/env python3
"""
Owner-run check of the planned macOS switch against REAL Claude logins (no Quayflow build needed).

It never touches your default Claude login (Keychain item "Claude Code-credentials").
Everything happens in a throwaway environment (a CLAUDE_CONFIG_DIR under $TMPDIR):

  1. You sign in twice with Claude's own sign-in: account 1 into the test environment,
     account 2 into a source folder. Use two DIFFERENT accounts.
  2. A loopback proxy forwards Claude's API calls to api.anthropic.com and records, per
     request, only the first 12 hex characters of SHA-256 of the bearer token.
  3. You open two interactive Claude sessions in the test environment.
  4. The script switches the test environment to account 2 the way Quayflow will on macOS:
     Claude's locks, /usr/bin/security with the item merged (other keys kept), a read-back,
     a marker .credentials.json holding the same login (owner-only), oauthAccount updated.
  5. It checks that both sessions and a headless run use account 2 from their next request,
     that removing the marker 35 s later changes nothing, that no Keychain dialog appeared
     and how long the locks were held.
  6. It deletes the test Keychain items and folders.

Prompts are synthetic ("reply with the word one") and use the Haiku model.
Run it yourself and paste the result table:  python3 tests/e2e/real-macos-keychain.py
Options: --keep (leave the test environment for inspection), --port N (proxy port, default 48300)
"""
import argparse
import hashlib
import http.client
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unicodedata
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

SECURITY = "/usr/bin/security"
DEFAULT_SERVICE = "Claude Code-credentials"
UPSTREAM = "api.anthropic.com"
MAX_STDIN_LINE = 4032  # Claude's own limit for `security -i`; above it Claude falls back to argv
LOCK_WAIT_S = 9.0
MARKER_LIFETIME_S = 35.0

RESULTS = []


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok), detail))
    print(f"  {'PASS' if ok else 'FAIL'}  {name}{('  (' + detail + ')') if detail else ''}")


def fp(token):
    return hashlib.sha256(token.encode()).hexdigest()[:12] if token else "-"


# ---------------------------------------------------------------- Keychain


def service_for(config_dir):
    """Claude's service name for a CLAUDE_CONFIG_DIR value: sha256 of the raw, NFC-normalized string."""
    raw = unicodedata.normalize("NFC", config_dir)
    return f"{DEFAULT_SERVICE}-{hashlib.sha256(raw.encode()).hexdigest()[:8]}"


def guard(service):
    if service == DEFAULT_SERVICE or not service.startswith(DEFAULT_SERVICE + "-"):
        sys.exit(f"refusing to touch Keychain service {service!r}: only test items are allowed")


SECURITY_TIMES = []


def run_security(args, stdin=None, timeout=20):
    t = time.time()
    try:
        p = subprocess.run([SECURITY, *args], input=stdin, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        SECURITY_TIMES.append((args[0] if args else "-i", timeout))
        return 124, "", "timed out (a Keychain dialog may be waiting)"
    SECURITY_TIMES.append((args[0] if args else "-i", time.time() - t))
    return p.returncode, p.stdout, p.stderr


def item_account(service):
    """The account attribute of an existing item (taken from the item, never computed)."""
    code, out, _ = run_security(["find-generic-password", "-s", service])
    if code != 0:
        return None
    for line in out.splitlines():
        line = line.strip()
        if line.startswith('"acct"<blob>='):
            v = line.split("=", 1)[1]
            return v[1:-1] if v.startswith('"') else None
    return None


def read_item(service, account):
    code, out, err = run_security(["find-generic-password", "-a", account, "-s", service, "-w"])
    if code != 0:
        return None
    return json.loads(out.strip())


def write_item(service, account, data):
    guard(service)
    hexdata = json.dumps(data, separators=(",", ":")).encode().hex()
    line = f'add-generic-password -U -a "{account}" -s "{service}" -X "{hexdata}"\n'
    if len(line) > MAX_STDIN_LINE:
        sys.exit("login payload too large for `security -i` stdin; refusing to put it on the command line")
    code, _, err = run_security(["-i"], stdin=line)
    return code == 0, err.strip()


def delete_item(service, account):
    guard(service)
    run_security(["delete-generic-password", "-a", account, "-s", service])


# ---------------------------------------------------------------- Claude's locks (proper-lockfile directories)


class Locks:
    """Takes Claude's locks in Quayflow's order; never removes a lock it did not create."""

    def __init__(self, config_dir):
        self.paths = [
            os.path.join(config_dir, ".oauth_refresh.lock"),
            os.path.realpath(config_dir) + ".lock",
            os.path.join(config_dir, ".storage-write.lock"),
            os.path.join(config_dir, ".claude.json") + ".lock",
        ]
        self.held = []

    def __enter__(self):
        for p in self.paths:
            end = time.time() + LOCK_WAIT_S
            while True:
                try:
                    os.mkdir(p)
                    self.held.append(p)
                    break
                except FileExistsError:
                    if time.time() > end:
                        self.__exit__()
                        raise RuntimeError(f"lock busy: {os.path.basename(p)}")
                    time.sleep(0.1)
        self.taken_at = time.time()
        return self

    def __exit__(self, *a):
        self.hold_s = time.time() - getattr(self, "taken_at", time.time())
        for p in reversed(self.held):
            try:
                os.rmdir(p)
            except OSError:
                pass
        self.held = []


# ---------------------------------------------------------------- files


def atomic_write(path, data, mode=0o600):
    d = os.path.dirname(path)
    tmp = os.path.join(d, f".qf-tmp-{os.getpid()}-{time.time_ns()}")
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, mode)
    with os.fdopen(fd, "wb") as f:
        f.write(data)
        f.flush()
        os.fsync(f.fileno())
    os.rename(tmp, path)


def load_json(path):
    with open(path, encoding="utf-8-sig") as f:
        return json.load(f)


# ---------------------------------------------------------------- recording proxy

LOG = []
LOG_LOCK = threading.Lock()
HOP = {"connection", "keep-alive", "proxy-connection", "transfer-encoding", "upgrade", "te", "trailer", "host", "content-length"}


class Proxy(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def _forward(self):
        n = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(n) if n else None
        auth = self.headers.get("Authorization", "")
        token = auth[7:] if auth.startswith("Bearer ") else ""
        sess = {k: v[:40] for k, v in self.headers.items() if "session" in k.lower()}
        entry = {"t": time.time(), "method": self.command, "path": self.path.split("?")[0], "fp": fp(token), "session": sess}
        headers = {k: v for k, v in self.headers.items() if k.lower() not in HOP}
        headers["Host"] = UPSTREAM
        conn = http.client.HTTPSConnection(UPSTREAM, timeout=600)
        try:
            conn.request(self.command, self.path, body=body, headers=headers)
            resp = conn.getresponse()
            entry["status"] = resp.status
            self.send_response(resp.status)
            length = resp.getheader("Content-Length")
            for k, v in resp.getheaders():
                if k.lower() not in HOP:
                    self.send_header(k, v)
            if length is not None:
                self.send_header("Content-Length", length)
            else:
                self.send_header("Connection", "close")
                self.close_connection = True
            self.end_headers()
            while True:
                chunk = resp.read1(65536) if hasattr(resp, "read1") else resp.read(65536)
                if not chunk:
                    break
                self.wfile.write(chunk)
                self.wfile.flush()
        except Exception as e:  # report and close; Claude sees a dropped connection
            entry["status"] = f"proxy error {type(e).__name__}"
            self.close_connection = True
        finally:
            conn.close()
            with LOG_LOCK:
                LOG.append(entry)

    do_GET = do_POST = do_PUT = do_DELETE = do_PATCH = _forward


def messages(since=0.0, until=None):
    with LOG_LOCK:
        return [e for e in LOG if e["path"].endswith("/v1/messages") and e["t"] >= since and (until is None or e["t"] < until)]


def describe(entries, names):
    by = {}
    for e in entries:
        name = names.get(e["fp"], f"other ({e['fp']})")
        by[name] = by.get(name, 0) + 1
    return ", ".join(f"{k}: {v}" for k, v in sorted(by.items())) or "no requests"


def sessions_of(entries):
    """Distinct X-Claude-Code-Session-Id values (Claude sends it on every request)."""
    return {json.dumps(e["session"], sort_keys=True) for e in entries if e["session"]}


def pause(msg):
    input(f"\n>>> {msg}\n    Press Enter when done. ")


def wait_for_sessions(since, count=2, timeout=180, settle=5.0):
    """Until `count` session ids have sent requests since `since`, then `settle` s for the answers."""
    end = time.time() + timeout
    while time.time() < end:
        if len(sessions_of(messages(since))) >= count:
            time.sleep(settle)
            return True
        time.sleep(0.5)
    return False


class ManualSessions:
    """You open the two sessions in your own terminal windows and type the prompts."""

    def open(self, launch, env, cwd):
        print(f"    Open TWO new terminal windows and run in each:\n\n    cd '{cwd}' && {launch}\n")

    def send_all(self, text, now=False):
        if now:
            print(f"    >>> Send in EACH session now:  {text}")
            pause(f"Wait until both sessions have answered '{text.split()[-1]}'")
        else:
            pause(f"In EACH session send:  {text}")

    def close(self):
        pause("Exit both sessions (/exit)")

    def screens(self):
        return []


class PtySessions:
    """Runs the two interactive sessions on pseudo-terminals and types the prompts (--auto)."""

    def __init__(self, claude):
        self.claude = claude
        self.procs = []

    def open(self, launch, env, cwd):
        import fcntl
        import pty
        import struct
        import termios

        for _ in range(2):
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 160, 0, 0))
            p = subprocess.Popen([self.claude, "--model", "haiku"], stdin=slave, stdout=slave, stderr=slave, cwd=cwd,
                                 env={**env, "TERM": "xterm-256color"}, start_new_session=True)
            os.close(slave)
            s = {"p": p, "fd": master, "buf": bytearray()}
            threading.Thread(target=self._drain, args=(s,), daemon=True).start()
            self.procs.append(s)
        time.sleep(8)
        for s in self.procs:  # accept a folder-trust question if one is shown
            if b"trust" in bytes(s["buf"][-20000:]).lower():
                os.write(s["fd"], b"\r")
        time.sleep(3)

    @staticmethod
    def _drain(s):
        while True:
            try:
                chunk = os.read(s["fd"], 65536)
            except OSError:
                return
            if not chunk:
                return
            s["buf"] += chunk
            del s["buf"][:-400000]

    def send_all(self, text, now=False):
        t = time.time()
        for s in self.procs:
            os.write(s["fd"], text.encode())
            time.sleep(0.4)
            os.write(s["fd"], b"\r")
        print(f"    typed in both sessions: {text}")
        if not wait_for_sessions(t):
            print("    (timed out waiting for both sessions)")

    def close(self):
        for s in self.procs:
            try:
                os.write(s["fd"], b"/exit\r")
            except OSError:
                pass
        time.sleep(3)
        for s in self.procs:
            if s["p"].poll() is None:
                s["p"].terminate()
            try:
                os.close(s["fd"])
            except OSError:
                pass

    def screens(self):
        import re
        out = []
        for s in self.procs:
            text = re.sub(rb"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07]*\x07|\x1b.", b"", bytes(s["buf"][-6000:]))
            out.append(text.decode("utf-8", "replace")[-1500:])
        return out


# ---------------------------------------------------------------- main


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--keep", action="store_true")
    ap.add_argument("--port", type=int, default=48300)
    ap.add_argument("--auto", action="store_true",
                    help="drive the two interactive sessions on pseudo-terminals; you only do the two browser sign-ins")
    a = ap.parse_args()
    ask = (lambda msg: print(msg)) if a.auto else input

    if sys.platform != "darwin":
        sys.exit("macOS only.")
    claude = shutil.which("claude")
    if not claude or not os.path.exists(SECURITY):
        sys.exit("needs `claude` on PATH and /usr/bin/security.")
    version = subprocess.run([claude, "--version"], capture_output=True, text=True).stdout.strip()
    print(f"Claude Code: {version}")

    work = tempfile.mkdtemp(prefix="qf-macos-real-")  # raw $TMPDIR path, kept as is (not resolved)
    env_dir = os.path.join(work, "env")
    src_dir = os.path.join(work, "source")
    os.makedirs(env_dir)
    os.makedirs(src_dir)
    env_svc, src_svc = service_for(env_dir), service_for(src_dir)
    guard(env_svc)
    guard(src_svc)
    proxy = None
    sessions = None
    try:
        print(f"\nTest environment: {env_dir}\nKeychain items:   {env_svc}, {src_svc}")
        print("\nStep 1: sign in. A browser window opens each time; finish the sign-in there.")
        ask(">>> Sign in with ACCOUNT 1 (the one to switch FROM). Press Enter to start. ")
        subprocess.run([claude, "auth", "login"], env={**os.environ, "CLAUDE_CONFIG_DIR": env_dir})
        ask(">>> Now sign in with ACCOUNT 2, a DIFFERENT account. Press Enter to start. ")
        subprocess.run([claude, "auth", "login"], env={**os.environ, "CLAUDE_CONFIG_DIR": src_dir})

        print("\nStep 2: logins")
        env_acct, src_acct = item_account(env_svc), item_account(src_svc)
        check("sign-in wrote the Keychain item named sha256(CLAUDE_CONFIG_DIR)", env_acct and src_acct,
              f"account attribute {env_acct!r}")
        if not (env_acct and src_acct):
            return
        env_item, src_item = read_item(env_svc, env_acct), read_item(src_svc, src_acct)
        a_tok, b_tok = env_item["claudeAiOauth"]["accessToken"], src_item["claudeAiOauth"]["accessToken"]
        a_acc = load_json(os.path.join(env_dir, ".claude.json")).get("oauthAccount", {})
        b_acc = load_json(os.path.join(src_dir, ".claude.json")).get("oauthAccount", {})
        check("two different accounts", a_acc.get("accountUuid") and a_acc.get("accountUuid") != b_acc.get("accountUuid"))
        if a_acc.get("accountUuid") == b_acc.get("accountUuid"):
            return
        print(f"  keys in the environment's item: {sorted(env_item)}")
        check("no .credentials.json after sign-in", not os.path.exists(os.path.join(env_dir, ".credentials.json")))
        names = {fp(a_tok): "account 1", fp(b_tok): "account 2"}

        # an extra key stands in for MCP logins and the other keys the item can hold
        env_item["quayflowProbe"] = {"kept": True}
        ok, err = write_item(env_svc, env_acct, env_item)
        check("probe key added to the environment's item", ok, err)

        proxy = ThreadingHTTPServer(("127.0.0.1", a.port), Proxy)
        threading.Thread(target=proxy.serve_forever, daemon=True).start()
        launch = f"CLAUDE_CONFIG_DIR='{env_dir}' ANTHROPIC_BASE_URL=http://127.0.0.1:{a.port} claude --model haiku"
        session_env = {**os.environ, "CLAUDE_CONFIG_DIR": env_dir, "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{a.port}"}
        proj = os.path.join(work, "project")
        os.makedirs(proj)
        cj = os.path.join(env_dir, ".claude.json")
        config = load_json(cj)  # skip first-run screens in the test environment only
        config["hasCompletedOnboarding"] = True
        config.setdefault("projects", {})
        for p in {proj, os.path.realpath(proj)}:
            config["projects"].setdefault(p, {})["hasTrustDialogAccepted"] = True
        atomic_write(cj, json.dumps(config, indent=2).encode(), os.stat(cj).st_mode & 0o777)
        sessions = PtySessions(claude) if a.auto else ManualSessions()

        print("\nStep 3: two interactive sessions on account 1")
        t_open = time.time()
        sessions.open(launch, session_env, proj)
        sessions.send_all("reply with the word one")
        before = messages(t_open)
        check("sessions used account 1 before the switch", before and all(e["fp"] == fp(a_tok) for e in before),
              describe(before, names))
        check("two sessions seen before the switch", len(sessions_of(before)) >= 2, f"{len(sessions_of(before))} session ids")

        print("\nStep 4: switch the environment to account 2 (Quayflow's planned macOS switch)")
        ask(">>> Press Enter to switch now; then send the next prompt in both sessions within 10 s. ")
        n_sec = len(SECURITY_TIMES)
        marker = os.path.join(env_dir, ".credentials.json")
        with Locks(env_dir) as locks:
            live = read_item(env_svc, env_acct)
            merged = dict(live)
            merged["claudeAiOauth"] = src_item["claudeAiOauth"]
            ok, err = write_item(env_svc, env_acct, merged)
            back = read_item(env_svc, env_acct)
            marker_bytes = json.dumps(merged, separators=(",", ":")).encode()
            if ok and back == merged:
                atomic_write(marker, marker_bytes, 0o600)
                cj = os.path.join(env_dir, ".claude.json")
                config = load_json(cj)
                config["oauthAccount"] = b_acc
                atomic_write(cj, json.dumps(config, indent=2).encode(), os.stat(cj).st_mode & 0o777)
        t_switch = time.time()
        check("Keychain write and read-back match", ok and back == merged, err)
        check("other keys kept (probe key survives)", (back or {}).get("quayflowProbe") == {"kept": True})
        check("locks held under 3 s", locks.hold_s < 3.0, f"{locks.hold_s:.2f} s")
        slow = [(c, round(s, 1)) for c, s in SECURITY_TIMES[n_sec:] if s > 3.0]
        check("no slow `security` call (a dialog would block it)", not slow, str(slow) if slow else "")
        st = os.stat(marker)
        check("marker is owner-only", (st.st_mode & 0o077) == 0, oct(st.st_mode & 0o777))
        sessions.send_all("reply with the word two", now=True)
        after = messages(t_switch)
        check("both sessions used account 2 after the switch", after and all(e["fp"] == fp(b_tok) for e in after),
              describe(after, names))
        check("both sessions sent requests after the switch", len(sessions_of(after)) >= 2, f"{len(sessions_of(after))} session ids")
        if after:
            print(f"  first request after the switch: +{after[0]['t'] - t_switch:.1f} s")

        print("\nStep 5: headless run")
        interactive = sessions_of(before) | sessions_of(after)
        t_h = time.time()
        r = subprocess.run([claude, "-p", "reply with the word three", "--model", "haiku"], capture_output=True, text=True,
                           env={**os.environ, "CLAUDE_CONFIG_DIR": env_dir, "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{a.port}"},
                           stdin=subprocess.DEVNULL, timeout=300)
        hl = [e for e in messages(t_h) if json.dumps(e["session"], sort_keys=True) not in interactive]
        check("headless run answered", r.returncode == 0, (r.stdout.strip() or r.stderr.strip())[:80])
        check("headless run used account 2", hl and all(e["fp"] == fp(b_tok) for e in hl), describe(hl, names))

        print("\nStep 6: remove the marker")
        wait = MARKER_LIFETIME_S - (time.time() - t_switch)
        if wait > 0:
            print(f"  waiting {wait:.0f} s")
            time.sleep(wait)
        with open(marker, "rb") as f:
            same = f.read() == marker_bytes
        check("marker unchanged by Claude (safe to remove)", same)
        if same:
            os.remove(marker)
        t_removed = time.time()
        sessions.send_all("reply with the word four")
        late = [e for e in messages(t_removed) if json.dumps(e["session"], sort_keys=True) in interactive]
        check("sessions stay on account 2 after the marker is gone", late and all(e["fp"] == fp(b_tok) for e in late),
              describe(late, names))
        final = read_item(env_svc, env_acct) or {}
        check("probe key still in the item at the end", final.get("quayflowProbe") == {"kept": True})

        if a.auto:
            slow_all = [(c, round(s, 1)) for c, s in SECURITY_TIMES if s > 3.0]
            check("no Keychain dialog (judged by `security` call times)", not slow_all, str(slow_all) if slow_all else "")
        else:
            answer = input("\n>>> Did any Keychain dialog appear during the test? [y/N] ").strip().lower()
            check("no Keychain dialog", answer not in ("y", "yes"))
        if not all(ok for _, ok, _ in RESULTS):
            for i, screen in enumerate(sessions.screens()):
                print(f"\n--- session {i + 1} screen (tail) ---\n{screen}")
        sessions.close()
        sessions = None
    finally:
        if sessions is not None:
            try:
                sessions.close()
            except Exception:
                pass
        if proxy:
            proxy.shutdown()
        if a.keep:
            print(f"\nKept: {work} and Keychain items {env_svc}, {src_svc}")
        else:
            for svc in (env_svc, src_svc):
                acct = item_account(svc)
                if acct:
                    delete_item(svc, acct)
            shutil.rmtree(work, ignore_errors=True)
            print("\nRemoved the test environment and its Keychain items. Your default login was not touched.")

    print("\nResult (paste this):")
    print(f"  Claude Code {version}, macOS {subprocess.run(['sw_vers', '-productVersion'], capture_output=True, text=True).stdout.strip()}")
    for name, ok, detail in RESULTS:
        print(f"  {'PASS' if ok else 'FAIL'}  {name}{('  (' + detail + ')') if detail else ''}")
    passed = sum(1 for _, ok, _ in RESULTS if ok)
    print(f"  {passed}/{len(RESULTS)} passed")
    sys.exit(0 if passed == len(RESULTS) else 1)


if __name__ == "__main__":
    main()
