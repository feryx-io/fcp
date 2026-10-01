"""A webhook (docs/DEVELOPER_GUIDE.md §3): the tenant's events posted to
your URL, each signed, and checked with verify_webhook before it is
believed."""

import json
import os
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer

from fcp_api.api.webhooks import webhooks_create_webhook, webhooks_test_webhook
from fcp_api.models import CreateWebhookRequest, TestWebhookRequest
from fcp_sdk import client, ok, verify_webhook


def run(url: str, token: str, host: str = "127.0.0.1", port: int = 0) -> dict:
    fcp = client(url, token)
    received = []
    secret = {}

    # Your receiver: refuse anything whose signature does not verify, or
    # that is older than five minutes (a replay).
    class Receiver(BaseHTTPRequestHandler):
        def do_POST(self):
            body = self.rfile.read(int(self.headers["Content-Length"]))
            if not verify_webhook(secret["value"], self.headers["X-FCP-Signature"], body):
                self.send_response(401)
                self.end_headers()
                return
            received.append(json.loads(body))
            self.send_response(200)
            self.end_headers()

        def log_message(self, *args):
            pass

    server = HTTPServer((host, port), Receiver)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        made = ok(
            webhooks_create_webhook.sync_detailed(
                client=fcp,
                body=CreateWebhookRequest(
                    url=f"http://{host}:{server.server_port}/fcp",
                    events=["call.*", "voicemail.transcribed"],
                ),
            )
        )
        secret["value"] = made.secret  # shown once, like a token's

        # Ask for a test delivery, signed like every other.
        outcome = ok(
            webhooks_test_webhook.sync_detailed(made.webhook.id, client=fcp, body=TestWebhookRequest())
        )
        return {
            "webhook": made.webhook.id,
            "delivered": outcome.delivered,
            "received": [event["event_type"] for event in received],
        }
    finally:
        server.shutdown()


if __name__ == "__main__":
    print(run(os.environ["FCP_URL"], os.environ["FCP_TOKEN"]))
