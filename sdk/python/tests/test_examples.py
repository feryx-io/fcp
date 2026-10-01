"""The guide's examples (docs/DEVELOPER_GUIDE.md), run against the live
stack fcp-admin's tests/sdks.rs starts: an admin, and a call manager with a
call up whose id is FCP_SDK_CALL_ID."""

import os
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "examples"))

import listening_flow  # noqa: E402
import live_call  # noqa: E402
import scoped_token  # noqa: E402
import webhook_receiver  # noqa: E402

URL = os.environ.get("FCP_SDK_URL")
TOKEN = os.environ.get("FCP_SDK_TOKEN")
CALL_ID = os.environ.get("FCP_SDK_CALL_ID")
live = pytest.mark.skipif(not (URL and TOKEN), reason="FCP_SDK_URL and FCP_SDK_TOKEN not set")
with_call = pytest.mark.skipif(not (URL and TOKEN and CALL_ID), reason="no call is up")


@live
def test_example_a_scoped_token():
    out = scoped_token.run(URL, TOKEN)
    assert isinstance(out["token"], str)
    assert isinstance(out["users"], list)


@live
def test_example_a_webhook_verified():
    out = webhook_receiver.run(URL, TOKEN)
    assert out["delivered"], out
    assert out["received"] == ["webhook.test"]


@live
def test_example_a_flow_that_listens_and_its_number():
    out = listening_flow.run(URL, TOKEN, flow_id="front-desk-py", number="+15125550172")
    assert out == {"flow": "front-desk-py", "version": 1, "number": "+15125550172"}


@with_call
def test_example_a_live_call_held_resumed_and_ended():
    assert live_call.run(URL, TOKEN, CALL_ID) == ["held", "resumed", "ended"]
