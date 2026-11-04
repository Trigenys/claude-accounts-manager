import os
import tempfile
import unittest
from base64 import urlsafe_b64encode

import cam_relay as r

KEY = urlsafe_b64encode(b"\x04" + b"k" * 64).decode().rstrip("=")
P256 = KEY
AUTH = urlsafe_b64encode(b"a" * 16).decode().rstrip("=")
SUB = {"endpoint": "https://fcm.googleapis.com/fcm/send/abc", "keys": {"p256dh": P256, "auth": AUTH}}
DEV = "0123456789abcdef0123456789abcdef"
SKEY = "s" * 64


class Clock:
    def __init__(self, t=1_000_000):
        self.t = t

    def __call__(self):
        return self.t


def entry(at, body="QUJD", **kw):
    e = {"at": at, "expires": at + 6 * 3600, "authorization": "vapid t=x, k=y", "ttl": 21600, "urgency": "high", "body": body}
    e.update(kw)
    return e


class Sender:
    def __init__(self, status=201):
        self.status, self.sent = status, []

    def __call__(self, e, endpoint):
        self.sent.append((endpoint, e["body"]))
        return self.status


class MailboxTest(unittest.TestCase):
    def setUp(self):
        self.clock = Clock()
        self.relay = r.Relay(send=Sender(), clock=self.clock)

    def call(self, method, path, body=None, key=None):
        headers = {"Authorization": f"Bearer {key}"} if key else {}
        return r.route(self.relay, method, path, headers, body)

    def test_store_once_and_take_once(self):
        self.assertEqual(self.call("POST", "/api/pair", {"code": "ABCDEFGH23", "vapidKey": KEY})[0], 201)
        self.assertEqual(self.call("POST", "/api/pair", {"code": "ABCDEFGH23", "vapidKey": KEY})[0], 409)
        self.assertEqual(self.call("GET", "/api/pair/ABCDEFGH23"), (200, {"vapidKey": KEY, "expires": self.clock.t + 600}))
        self.assertEqual(self.call("POST", "/api/pair/ABCDEFGH23/take")[0], 204, "pending")
        self.assertEqual(self.call("PUT", "/api/pair/ABCDEFGH23", {"subscription": SUB, "label": "Pixel\n"})[0], 201)
        self.assertEqual(self.call("PUT", "/api/pair/ABCDEFGH23", {"subscription": SUB})[0], 409, "store once")
        self.assertEqual(self.call("GET", "/api/pair/ABCDEFGH23")[0], 404, "used offer hidden")
        status, out = self.call("POST", "/api/pair/ABCDEFGH23/take")
        self.assertEqual((status, out["label"], out["subscription"]), (200, "Pixel", SUB))
        self.assertEqual(self.call("POST", "/api/pair/ABCDEFGH23/take")[0], 404, "taken once")

    def test_mailbox_expires(self):
        self.call("POST", "/api/pair", {"code": "ABCDEFGH23", "vapidKey": KEY})
        self.clock.t += 601
        self.assertEqual(self.call("GET", "/api/pair/ABCDEFGH23")[0], 404)
        self.assertEqual(self.call("PUT", "/api/pair/ABCDEFGH23", {"subscription": SUB})[0], 404)
        self.assertEqual(self.relay.mailboxes, {})

    def test_rejects_bad_input_and_foreign_endpoints(self):
        self.assertEqual(self.call("POST", "/api/pair", {"code": "abc", "vapidKey": KEY})[0], 400)
        self.assertEqual(self.call("POST", "/api/pair", {"code": "ABCDEFGH23", "vapidKey": "short"})[0], 400)
        self.call("POST", "/api/pair", {"code": "ABCDEFGH23", "vapidKey": KEY})
        for endpoint in ("http://fcm.googleapis.com/x", "https://evil.example/x", "https://fcm.googleapis.com:8443/x",
                         "https://u@fcm.googleapis.com/x", "https://169.254.169.254/latest"):
            sub = dict(SUB, endpoint=endpoint)
            self.assertEqual(self.call("PUT", "/api/pair/ABCDEFGH23", {"subscription": sub})[0], 400, endpoint)
        self.assertTrue(r.allowed_endpoint("https://web.push.apple.com/Q"))
        self.assertTrue(r.allowed_endpoint("https://wns2-par02p.notify.windows.com/w/?token=x"))
        self.assertEqual(self.call("GET", "/elsewhere")[0], 404)


class ScheduleTest(unittest.TestCase):
    def setUp(self):
        self.clock = Clock()
        self.sender = Sender()
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, "state.json")
        self.relay = r.Relay(self.path, send=self.sender, clock=self.clock)

    def tearDown(self):
        self.dir.cleanup()

    def put(self, entries, key=SKEY, endpoint=SUB["endpoint"]):
        return r.route(self.relay, "PUT", f"/api/devices/{DEV}/schedule", {"Authorization": f"Bearer {key}"},
                       {"endpoint": endpoint, "entries": entries})

    def test_schedule_is_replaced_whole_and_persisted(self):
        now = self.clock.t
        self.assertEqual(self.put([entry(now + 100, "QQ"), entry(now + 50, "Qg")]), (200, {"stored": 2}))
        self.assertEqual(self.put([entry(now + 200, "Qw")]), (200, {"stored": 1}))
        reloaded = r.Relay(self.path, send=self.sender, clock=self.clock)
        self.assertEqual([e["body"] for e in reloaded.devices[DEV]["entries"]], ["Qw"])
        with open(self.path, encoding="utf-8") as f:
            self.assertNotIn(SKEY, f.read(), "only the key hash is stored")
        self.clock.t = now + 150
        self.assertEqual(self.relay.run_due(), 0, "replaced entries never fire")
        self.clock.t = now + 200
        self.assertEqual(self.relay.run_due(), 1)
        self.assertEqual(self.sender.sent, [(SUB["endpoint"], "Qw")])
        self.assertEqual(self.relay.run_due(), 0, "delivered once")
        self.assertEqual(self.relay.devices[DEV]["entries"], [])

    def test_key_is_bound_on_first_use(self):
        self.put([])
        self.assertEqual(self.put([], key="t" * 64)[0], 401)
        self.assertEqual(r.route(self.relay, "DELETE", f"/api/devices/{DEV}", {"Authorization": "Bearer " + "t" * 64}, None)[0], 401)
        self.assertEqual(r.route(self.relay, "DELETE", f"/api/devices/{DEV}", {"Authorization": f"Bearer {SKEY}"}, None)[0], 204)
        self.assertNotIn(DEV, self.relay.devices)
        self.assertEqual(r.route(self.relay, "DELETE", f"/api/devices/{DEV}", {"Authorization": f"Bearer {SKEY}"}, None)[0], 204, "idempotent")

    def test_gone_subscription_is_pruned_and_tombstoned(self):
        self.put([entry(self.clock.t)])
        self.sender.status = 410
        self.relay.run_due()
        self.assertNotIn(DEV, self.relay.devices)
        self.assertEqual(self.put([entry(self.clock.t + 10)])[0], 410, "the desktop learns it is gone")
        self.clock.t += r.TOMBSTONE_TTL + 1
        self.relay.run_due()
        self.assertEqual(self.put([])[0], 200, "tombstone expires")

    def test_transient_failure_retries_until_expiry(self):
        now = self.clock.t
        self.put([entry(now)])
        self.sender.status = 503
        self.assertEqual(self.relay.run_due(), 1)
        self.assertEqual(self.relay.run_due(), 0, "backoff")
        self.clock.t = now + r.RETRY_SECS
        self.sender.status = 201
        self.assertEqual(self.relay.run_due(), 1)
        self.assertEqual(self.relay.devices[DEV]["entries"], [])
        self.put([entry(now + 10)])
        self.sender.status = 0
        self.clock.t = now + 6 * 3600 + 11
        self.relay.run_due()
        self.assertEqual(self.relay.devices[DEV]["entries"], [], "expired entries are dropped")

    def test_invalid_schedules_are_rejected(self):
        now = self.clock.t
        bad = [
            [entry(now, expires=now + 2 * 86400)],
            [entry(now, authorization="Bearer x")],
            [entry(now, urgency="max")],
            [entry(now, body="!!")],
            [entry(now, topic="a b")],
            [entry(now + i) for i in range(r.MAX_ENTRIES + 1)],
        ]
        for entries in bad:
            self.assertEqual(self.put(entries)[0], 400)
        self.assertEqual(self.put([], endpoint="https://evil.example/x")[0], 400)
        self.assertEqual(self.put([], key="short")[0], 401)
        self.assertEqual(r.route(self.relay, "PUT", "/api/devices/NOT-AN-ID/schedule", {"Authorization": f"Bearer {SKEY}"}, {"endpoint": SUB["endpoint"], "entries": []})[0], 401)

    def test_rate_limiter(self):
        t = Clock(0)
        lim = r.RateLimiter(per_min=2, clock=t)
        self.assertTrue(lim.allow("1.2.3.4") and lim.allow("1.2.3.4"))
        self.assertFalse(lim.allow("1.2.3.4"))
        self.assertTrue(lim.allow("5.6.7.8"))
        t.t = 61
        self.assertTrue(lim.allow("1.2.3.4"))


if __name__ == "__main__":
    unittest.main()
