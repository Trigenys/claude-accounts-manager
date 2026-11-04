#!/usr/bin/env python3
"""Push relay for Trigenys Claude Accounts Manager (Python 3 standard library only).

Two jobs, nothing else:
- pairing mailbox: the desktop opens an offer (its VAPID public key) under a short code, the phone
  answers once with its PushSubscription, the desktop takes the answer once; everything expires in minutes;
- reset scheduler: each paired phone has a schedule of pre-encrypted pushes (opaque aes128gcm bodies and
  pre-signed VAPID headers) that the relay POSTs to the phone's push endpoint at their time, so resets reach
  the phone with the PC off. The desktop replaces a phone's schedule whole; entries expire after the reset.

Stored: subscription endpoint, SHA-256 of the schedule key, reset times, opaque payloads. Never quotas,
percentages, VAPID private keys or anything from Claude. Listens on 127.0.0.1 behind Caddy.
"""

import hashlib
import hmac
import json
import os
import re
import sys
import threading
import time
import urllib.error
import urllib.request
from base64 import urlsafe_b64decode
from collections import defaultdict, deque
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit

PAIR_TTL = 10 * 60
TOMBSTONE_TTL = 30 * 24 * 3600
IDLE_DEVICE_TTL = 60 * 24 * 3600
MAX_ENTRY_LIFE = 24 * 3600  # VAPID tokens are valid at most 24 h after sending
MAX_BODY = 64 * 1024
MAX_ENTRIES = 32
MAX_DEVICES = 1000
MAX_MAILBOXES = 500
RATE_PER_MIN = 60
RETRY_SECS = 60

CODE_RE = re.compile(r"^[A-HJ-NP-Z2-9]{10}$")
ID_RE = re.compile(r"^[0-9a-f]{32}$")
B64_RE = re.compile(r"^[A-Za-z0-9_-]+={0,2}$")
PUSH_HOSTS = ("fcm.googleapis.com", "updates.push.services.mozilla.com", "web.push.apple.com")
PUSH_SUFFIXES = (".push.apple.com", ".notify.windows.com")


def allowed_endpoint(url):
    if not isinstance(url, str) or len(url) > 1024:
        return False
    u = urlsplit(url)
    host = u.hostname or ""
    return (
        u.scheme == "https"
        and u.port is None
        and not u.username
        and (host in PUSH_HOSTS or host.endswith(PUSH_SUFFIXES))
    )


def b64_len(s):
    if not isinstance(s, str) or not B64_RE.match(s):
        return -1
    try:
        return len(urlsafe_b64decode(s + "=" * (-len(s) % 4)))
    except ValueError:
        return -1


def key_hash(key):
    return hashlib.sha256(key.encode()).hexdigest()


class Error(Exception):
    def __init__(self, status, message):
        super().__init__(message)
        self.status = status
        self.message = message


def http_send(entry, endpoint):
    """POSTs one pre-encrypted push; returns the HTTP status (0 on network error)."""
    body = urlsafe_b64decode(entry["body"] + "=" * (-len(entry["body"]) % 4))
    headers = {
        "Authorization": entry["authorization"],
        "TTL": str(entry["ttl"]),
        "Urgency": entry["urgency"],
        "Content-Encoding": "aes128gcm",
        "Content-Type": "application/octet-stream",
    }
    if entry.get("topic"):
        headers["Topic"] = entry["topic"]
    req = urllib.request.Request(endpoint, data=body, headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            return r.status
    except urllib.error.HTTPError as e:
        return e.code
    except (urllib.error.URLError, OSError):
        return 0


class Relay:
    def __init__(self, path=None, send=http_send, clock=time.time, log=None):
        self.path = path
        self.log = log or (lambda msg: print(msg, flush=True))
        self.send = send
        self.clock = clock
        self.lock = threading.Lock()
        self.mailboxes = {}  # memory only: minutes-long
        self.devices = {}
        self.tombstones = {}
        if path and os.path.exists(path):
            with open(path, encoding="utf-8") as f:
                state = json.load(f)
            self.devices = state.get("devices", {})
            self.tombstones = state.get("tombstones", {})

    def now(self):
        return int(self.clock())

    def save(self):
        if not self.path:
            return
        tmp = self.path + ".tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump({"devices": self.devices, "tombstones": self.tombstones}, f)
        os.replace(tmp, self.path)

    # ---------- pairing mailbox ----------

    def gc_mailboxes(self, now):
        for c in [c for c, m in self.mailboxes.items() if m["expires"] <= now]:
            del self.mailboxes[c]

    def offer(self, body):
        code, key = body.get("code"), body.get("vapidKey")
        if not isinstance(code, str) or not CODE_RE.match(code) or b64_len(key) != 65:
            raise Error(400, "invalid offer")
        with self.lock:
            now = self.now()
            self.gc_mailboxes(now)
            if code in self.mailboxes:
                raise Error(409, "code in use")
            if len(self.mailboxes) >= MAX_MAILBOXES:
                raise Error(503, "busy")
            self.mailboxes[code] = {"vapidKey": key, "answer": None, "expires": now + PAIR_TTL}
        return 201, {"expires": now + PAIR_TTL}

    def get_offer(self, code):
        with self.lock:
            self.gc_mailboxes(self.now())
            m = self.mailboxes.get(code)
            if not m or m["answer"] is not None:
                raise Error(404, "unknown or used code")
            return 200, {"vapidKey": m["vapidKey"], "expires": m["expires"]}

    def answer(self, code, body):
        sub = body.get("subscription") if isinstance(body, dict) else None
        keys = sub.get("keys") if isinstance(sub, dict) else None
        if (
            not isinstance(keys, dict)
            or not allowed_endpoint(sub.get("endpoint"))
            or b64_len(keys.get("p256dh")) != 65
            or b64_len(keys.get("auth")) != 16
        ):
            raise Error(400, "invalid subscription")
        label = body.get("label") if isinstance(body.get("label"), str) else ""
        clean = {
            "subscription": {
                "endpoint": sub["endpoint"],
                "keys": {"p256dh": keys["p256dh"], "auth": keys["auth"]},
            },
            "label": "".join(ch for ch in label if ch.isprintable())[:40],
        }
        with self.lock:
            self.gc_mailboxes(self.now())
            m = self.mailboxes.get(code)
            if not m:
                raise Error(404, "unknown or expired code")
            if m["answer"] is not None:
                raise Error(409, "already answered")
            m["answer"] = clean
        return 201, {}

    def take(self, code):
        with self.lock:
            self.gc_mailboxes(self.now())
            m = self.mailboxes.get(code)
            if not m:
                raise Error(404, "unknown or expired code")
            if m["answer"] is None:
                return 204, None
            del self.mailboxes[code]
            return 200, m["answer"]

    # ---------- schedules ----------

    def owned(self, device_id, key, create=False):
        """The device, if `key` owns it. Trust on first use: the first PUT registers the key."""
        if not ID_RE.match(device_id or "") or not isinstance(key, str) or len(key) < 32:
            raise Error(401, "unauthorized")
        if device_id in self.tombstones:
            raise Error(410, "subscription gone")
        d = self.devices.get(device_id)
        if d is None:
            if not create:
                raise Error(404, "unknown device")
            if len(self.devices) >= MAX_DEVICES:
                raise Error(503, "busy")
            return None
        if not hmac.compare_digest(d["keyHash"], key_hash(key)):
            raise Error(401, "unauthorized")
        return d

    def put_schedule(self, device_id, key, body):
        endpoint = body.get("endpoint") if isinstance(body, dict) else None
        entries = body.get("entries") if isinstance(body, dict) else None
        if not allowed_endpoint(endpoint) or not isinstance(entries, list) or len(entries) > MAX_ENTRIES:
            raise Error(400, "invalid schedule")
        now = self.now()
        clean = []
        for e in entries:
            ok = (
                isinstance(e, dict)
                and isinstance(e.get("at"), int)
                and isinstance(e.get("expires"), int)
                and e["at"] < e["expires"] <= e["at"] + MAX_ENTRY_LIFE
                and isinstance(e.get("authorization"), str)
                and e["authorization"].startswith("vapid t=")
                and len(e["authorization"]) < 1024
                and isinstance(e.get("ttl"), int)
                and 0 <= e["ttl"] <= MAX_ENTRY_LIFE
                and e.get("urgency") in ("very-low", "low", "normal", "high")
                and (e.get("topic") is None or re.match(r"^[A-Za-z0-9_-]{1,32}$", str(e["topic"])))
                and 0 < b64_len(e.get("body")) <= 4096
            )
            if not ok:
                raise Error(400, "invalid entry")
            if e["expires"] > now:
                clean.append({k: e[k] for k in ("at", "expires", "authorization", "ttl", "urgency", "topic", "body") if e.get(k) is not None})
        with self.lock:
            d = self.owned(device_id, key, create=True)
            if d is None:
                d = self.devices[device_id] = {"keyHash": key_hash(key)}
            d.update(endpoint=endpoint, entries=sorted(clean, key=lambda e: e["at"]), updated=now)
            self.save()
        return 200, {"stored": len(clean)}

    def forget(self, device_id, key):
        with self.lock:
            try:
                self.owned(device_id, key)
            except Error as e:
                if e.status in (404, 410):
                    return 204, None
                raise
            del self.devices[device_id]
            self.save()
        return 204, None

    def prune(self, device_id, now):
        self.devices.pop(device_id, None)
        self.tombstones[device_id] = now + TOMBSTONE_TTL

    def run_due(self):
        """Sends every due entry once; keeps transient failures for a retry until the entry expires."""
        with self.lock:
            now = self.now()
            due = [
                (i, d["endpoint"], e)
                for i, d in self.devices.items()
                for e in d.get("entries", [])
                if e["at"] <= now and e.get("retry", 0) <= now
            ]
        results = [(i, e, self.send(e, endpoint)) for i, endpoint, e in due]
        with self.lock:
            now = self.now()
            changed = False
            for i, e, status in results:
                self.log(f"push {i[:8]} {status}")  # outcome only: no endpoint, keys or payload
                d = self.devices.get(i)
                if d is None or e not in d.get("entries", []):
                    continue  # replaced or revoked meanwhile
                if status in (404, 410):
                    self.prune(i, now)
                elif status == 0 or status == 429 or status >= 500:
                    e["retry"] = now + RETRY_SECS
                else:
                    d["entries"].remove(e)  # delivered, or refused for good
                changed = True
            for i, d in list(self.devices.items()):
                live = [e for e in d.get("entries", []) if e["expires"] > now]
                if live != d.get("entries", []):
                    d["entries"] = live
                    changed = True
                if not live and d.get("updated", 0) + IDLE_DEVICE_TTL <= now:
                    del self.devices[i]
                    changed = True
            for i in [i for i, t in self.tombstones.items() if t <= now]:
                del self.tombstones[i]
                changed = True
            if changed:
                self.save()
        return len(results)


class RateLimiter:
    def __init__(self, per_min=RATE_PER_MIN, clock=time.monotonic):
        self.per_min, self.clock, self.hits, self.lock = per_min, clock, defaultdict(deque), threading.Lock()

    def allow(self, ip):
        with self.lock:
            now, q = self.clock(), self.hits[ip]
            while q and q[0] <= now - 60:
                q.popleft()
            if len(q) >= self.per_min:
                return False
            q.append(now)
            if len(self.hits) > 10000:
                self.hits = defaultdict(deque, {ip: q})
            return True


def route(relay, method, path, headers, body):
    """Pure request dispatch: returns (status, json or None)."""
    parts = path.rstrip("/").split("/")[1:]  # api, ...
    bearer = headers.get("Authorization", "")
    key = bearer[7:] if bearer.startswith("Bearer ") else None
    try:
        if parts[:2] != ["api", "pair"] and parts[:2] != ["api", "devices"]:
            raise Error(404, "not found")
        if parts == ["api", "pair"] and method == "POST":
            return relay.offer(body or {})
        if len(parts) >= 3 and parts[1] == "pair":
            code = parts[2]
            if not CODE_RE.match(code):
                raise Error(404, "unknown code")
            if len(parts) == 3 and method == "GET":
                return relay.get_offer(code)
            if len(parts) == 3 and method == "PUT":
                return relay.answer(code, body or {})
            if parts[3:] == ["take"] and method == "POST":
                return relay.take(code)
        if len(parts) >= 3 and parts[1] == "devices":
            if parts[3:] == ["schedule"] and method == "PUT":
                return relay.put_schedule(parts[2], key, body or {})
            if len(parts) == 3 and method == "DELETE":
                return relay.forget(parts[2], key)
        raise Error(405, "method not allowed")
    except Error as e:
        return e.status, {"error": e.message}


def make_handler(relay, limiter):
    class Handler(BaseHTTPRequestHandler):
        server_version = "cam-relay"
        sys_version = ""

        def handle_any(self):
            ip = (self.headers.get("X-Forwarded-For") or self.client_address[0]).split(",")[-1].strip()
            if not limiter.allow(ip):
                return self.reply(429, {"error": "too many requests"})
            length = int(self.headers.get("Content-Length") or 0)
            if length > MAX_BODY:
                return self.reply(413, {"error": "too large"})
            body = None
            if length:
                try:
                    body = json.loads(self.rfile.read(length))
                except ValueError:
                    return self.reply(400, {"error": "invalid json"})
            status, out = route(relay, self.command, self.path.split("?")[0], self.headers, body)
            self.reply(status, out)

        def reply(self, status, out):
            data = b"" if out is None or status == 204 else json.dumps(out).encode()
            self.send_response(status)
            self.send_header("Cache-Control", "no-store")
            if data:
                self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        do_GET = do_POST = do_PUT = do_DELETE = handle_any

        def log_message(self, fmt, *args):  # no paths (codes) or IPs in logs
            pass

    return Handler


def main():
    port = int(os.environ.get("CAM_RELAY_PORT", "8787"))
    state_dir = os.environ.get("STATE_DIRECTORY", ".")
    relay = Relay(os.path.join(state_dir, "state.json"))

    def scheduler():
        while True:
            try:
                relay.run_due()
            except Exception as e:  # keep the loop alive; never log payloads
                print(f"scheduler error: {type(e).__name__}", file=sys.stderr, flush=True)
            time.sleep(10)

    threading.Thread(target=scheduler, daemon=True).start()
    server = ThreadingHTTPServer(("127.0.0.1", port), make_handler(relay, RateLimiter()))
    print(f"cam-relay listening on 127.0.0.1:{port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
