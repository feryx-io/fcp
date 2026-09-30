"""Against a running admin: fcp-admin's tests/sdks.rs starts one on a
throwaway database and runs this with FCP_SDK_URL and FCP_SDK_TOKEN."""

import os

import pytest

from fcp_api.api.api_tokens import api_tokens_create_api_token
from fcp_api.api.devices import devices_list_devices
from fcp_api.api.users import users_create_user, users_get_user, users_list_users
from fcp_api.models import CreateApiTokenRequest, CreateUserRequest
from fcp_sdk import FcpError, client, ok, paginate

URL = os.environ.get("FCP_SDK_URL")
TOKEN = os.environ.get("FCP_SDK_TOKEN")
live = pytest.mark.skipif(not (URL and TOKEN), reason="FCP_SDK_URL and FCP_SDK_TOKEN not set")


@live
def test_lists_page_by_cursor_until_every_item_is_read():
    fcp = client(URL, TOKEN)
    names = ["py-a", "py-b", "py-c", "py-d", "py-e"]
    for username in names:
        ok(
            users_create_user.sync_detailed(
                client=fcp,
                body=CreateUserRequest(username=username, password="Str0ng-Passw0rd!", role="end_user"),
            )
        )
    seen = [u.username for u in paginate(users_list_users.sync_detailed, client=fcp, limit=2)]
    for n in names:
        assert n in seen
    assert len(seen) == len(set(seen))


@live
def test_a_refusal_is_an_fcp_error_with_the_code_and_a_request_id():
    fcp = client(URL, TOKEN)
    with pytest.raises(FcpError) as err:
        ok(users_get_user.sync_detailed(client=fcp, username="nobody-at-all"))
    assert err.value.status == 404
    assert err.value.code == "not_found"
    assert err.value.request_id
    with pytest.raises(FcpError) as err:
        ok(users_list_users.sync_detailed(client=client(URL, "fcp_not-a-token")))
    assert err.value.code == "unauthorized"


@live
def test_a_scoped_token_made_through_the_sdk_reaches_what_it_names():
    fcp = client(URL, TOKEN)
    made = ok(
        api_tokens_create_api_token.sync_detailed(
            client=fcp, body=CreateApiTokenRequest(name="py-sdk", permissions=["user_read"])
        )
    )
    scoped = client(URL, made.secret)
    assert isinstance(ok(users_list_users.sync_detailed(client=scoped)).data, list)
    with pytest.raises(FcpError) as err:
        ok(devices_list_devices.sync_detailed(client=scoped))
    assert err.value.code == "forbidden"


@live
def test_a_webhook_delivery_verifies_and_a_replay_later_is_stale():
    import json
    import re
    import threading
    from http.server import BaseHTTPRequestHandler, HTTPServer

    from fcp_api.api.webhooks import (
        webhooks_create_webhook,
        webhooks_delete_webhook,
        webhooks_test_webhook,
    )
    from fcp_api.models import CreateWebhookRequest, TestWebhookRequest
    from fcp_sdk import verify_webhook

    seen = []

    class Hook(BaseHTTPRequestHandler):
        def do_POST(self):
            body = self.rfile.read(int(self.headers["Content-Length"]))
            seen.append((self.headers["X-FCP-Signature"], body))
            self.send_response(200)
            self.end_headers()

        def log_message(self, *args):
            pass

    server = HTTPServer(("127.0.0.1", 0), Hook)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        fcp = client(URL, TOKEN)
        made = ok(
            webhooks_create_webhook.sync_detailed(
                client=fcp,
                body=CreateWebhookRequest(url=f"http://127.0.0.1:{server.server_port}/hook"),
            )
        )
        assert made.secret.startswith("whsec_")
        outcome = ok(
            webhooks_test_webhook.sync_detailed(made.webhook.id, client=fcp, body=TestWebhookRequest())
        )
        assert outcome.delivered
        signature, body = seen[-1]
        assert verify_webhook(made.secret, signature, body)
        assert json.loads(body)["event_type"] == "webhook.test"
        t = int(re.search(r"t=(\d+)", signature).group(1))
        assert not verify_webhook(made.secret, signature, body, now=t + 600)
        assert not verify_webhook("whsec_wrong", signature, body)
        webhooks_delete_webhook.sync_detailed(made.webhook.id, client=fcp)
    finally:
        server.shutdown()
