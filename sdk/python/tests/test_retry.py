"""Retries, errors and headers against a local stand-in for the API."""

import json
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer

import pytest

from fcp_api.api.users import users_get_user, users_list_users
from fcp_api.models import CreateUserRequest
from fcp_sdk import FcpError, client, ok

LIMITED = (429, {"error": "Too Many Requests", "code": "rate_limited", "details": "slow down"}, {"retry-after": "0"})


class StandIn:
    def __init__(self, answers):
        self.answers = list(answers)
        self.seen = []
        outer = self

        class Handler(BaseHTTPRequestHandler):
            def _answer(self):
                length = int(self.headers.get("content-length") or 0)
                body = self.rfile.read(length).decode()
                outer.seen.append({"path": self.path, "body": body, "headers": dict(self.headers)})
                status, payload, headers = outer.answers.pop(0) if outer.answers else (500, {}, {})
                data = json.dumps(payload).encode()
                self.send_response(status)
                self.send_header("content-type", "application/json")
                self.send_header("x-request-id", "req-1")
                self.send_header("content-length", str(len(data)))
                for k, v in headers.items():
                    self.send_header(k, v)
                self.end_headers()
                self.wfile.write(data)

            do_GET = do_POST = _answer

            def log_message(self, *args):
                pass

        self.server = HTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self):
        self.server.shutdown()


def test_a_429_is_retried_and_the_body_sent_again_each_time():
    srv = StandIn([LIMITED, LIMITED, (201, {"made": True}, {})])
    try:
        fcp = client(srv.url, "fcp_t", tenant_id="tenant-x")
        # Through the client's own transport: what every operation sends.
        body = CreateUserRequest(username="ada", password="pw-123456789").to_dict()
        response = fcp.get_httpx_client().post("/api/v1/users", json=body)
        assert response.status_code == 201
        assert len(srv.seen) == 3
        for s in srv.seen:
            assert json.loads(s["body"])["username"] == "ada"
            assert s["headers"]["Authorization"] == "Bearer fcp_t"
            assert s["headers"]["X-FCP-Tenant-Id"] == "tenant-x"
    finally:
        srv.close()


def test_past_its_retries_the_api_error_is_raised():
    srv = StandIn([LIMITED, LIMITED])
    try:
        fcp = client(srv.url, "fcp_t", retries=1)
        with pytest.raises(FcpError) as err:
            ok(users_list_users.sync_detailed(client=fcp))
        assert err.value.status == 429
        assert err.value.code == "rate_limited"
        assert err.value.details == "slow down"
        assert err.value.request_id == "req-1"
        assert len(srv.seen) == 2
    finally:
        srv.close()


def test_any_other_error_is_not_retried():
    srv = StandIn([(404, {"error": "Not Found", "code": "not_found", "details": "no such user"}, {})])
    try:
        fcp = client(srv.url, "fcp_t")
        with pytest.raises(FcpError) as err:
            ok(users_get_user.sync_detailed(client=fcp, username="nobody"))
        assert err.value.code == "not_found"
        assert len(srv.seen) == 1
    finally:
        srv.close()
