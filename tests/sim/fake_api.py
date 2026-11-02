"""Fake Claude usage and profile API for simulation tests (standard library only).

Serves the two endpoints the Quayflow engine calls, keyed by the bearer token:

    GET /api/oauth/usage     five_hour, seven_day and per-model windows
    GET /api/oauth/profile   account and organization identity

Tests script it over loopback:

    POST /__sim/accounts     {"<access token>": {fields...}}   merge per-token state
    GET  /__sim/log          request counts per token and path
    POST /__sim/reset-log

Per-token fields (all optional except uuid):
    uuid, org, email, org_type            identity returned by /profile
    five_hour, seven_day                  utilization in percent (null = window absent)
    five_hour_reset_s, seven_day_reset_s  seconds from now until the window resets
    models                                [{"name": "Opus", "percent": 40, "reset_s": 3600}]
    status                                HTTP status for /usage (200 default; 429, 401, 500 ...)
    retry_after                           Retry-After seconds sent with a 429
    profile_status                        HTTP status for /profile (200 default)

Unknown tokens get 401, as the real service does. The server binds 127.0.0.1 only and prints
"PORT <n>" on its first output line.

Usage: python -I tests/sim/fake_api.py [--port N]
"""

import json
import sys
import threading
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

STATE: dict = {}
LOG: dict = {}
LOCK = threading.Lock()


def iso_in(seconds):
    if seconds is None:
        return None
    t = datetime.now(timezone.utc) + timedelta(seconds=float(seconds))
    return t.isoformat(timespec="milliseconds")


def window(pct, reset_s):
    if pct is None:
        return None
    return {"utilization": pct, "resets_at": iso_in(reset_s if reset_s is not None else 3600)}


class Handler(BaseHTTPRequestHandler):
    server_version = "quayflow-fake-api/1"

    def log_message(self, fmt, *args):  # keep test output quiet
        pass

    def send_json(self, code, body, headers=None):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        for k, v in (headers or {}).items():
            self.send_header(k, str(v))
        self.end_headers()
        self.wfile.write(data)

    def token(self):
        auth = self.headers.get("Authorization", "")
        return auth[7:] if auth.startswith("Bearer ") else None

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if path == "/__sim/log":
            with LOCK:
                return self.send_json(200, LOG)
        if path not in ("/api/oauth/usage", "/api/oauth/profile"):
            return self.send_json(404, {"type": "error", "error": {"type": "not_found_error"}})
        tok = self.token()
        with LOCK:
            LOG.setdefault(tok or "-", {}).setdefault(path, 0)
            LOG[tok or "-"][path] += 1
            acct = dict(STATE.get(tok) or {})
        if not acct:
            return self.send_json(401, {"type": "error", "error": {"type": "authentication_error", "message": "Invalid bearer token"}})
        if path == "/api/oauth/profile":
            code = int(acct.get("profile_status", 200))
            if code != 200:
                return self.send_json(code, {"type": "error", "error": {"type": "api_error"}})
            return self.send_json(200, {
                "account": {"uuid": acct["uuid"], "email": acct.get("email", acct["uuid"] + "@example.test")},
                "organization": {"uuid": acct.get("org", "org-" + acct["uuid"]), "organization_type": acct.get("org_type", "claude_max")},
            })
        code = int(acct.get("status", 200))
        if code == 429:
            return self.send_json(429, {"type": "error", "error": {"type": "rate_limit_error"}},
                                  {"retry-after": acct.get("retry_after", 5)})
        if code != 200:
            return self.send_json(code, {"type": "error", "error": {"type": "api_error"}})
        limits = [{"scope": {"model": {"display_name": m["name"]}}, "percent": m["percent"],
                   "resets_at": iso_in(m.get("reset_s", 3600))} for m in acct.get("models", [])]
        return self.send_json(200, {
            "five_hour": window(acct.get("five_hour"), acct.get("five_hour_reset_s")),
            "seven_day": window(acct.get("seven_day"), acct.get("seven_day_reset_s")),
            "limits": limits,
        })

    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(n) or b"{}")
        if self.path == "/__sim/accounts":
            with LOCK:
                for tok, fields in body.items():
                    STATE.setdefault(tok, {}).update(fields)
            return self.send_json(200, {"ok": True})
        if self.path == "/__sim/reset-log":
            with LOCK:
                LOG.clear()
            return self.send_json(200, {"ok": True})
        return self.send_json(404, {})


def main():
    port = 0
    if "--port" in sys.argv:
        port = int(sys.argv[sys.argv.index("--port") + 1])
    srv = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    print(f"PORT {srv.server_address[1]}", flush=True)
    srv.serve_forever()


if __name__ == "__main__":
    main()
