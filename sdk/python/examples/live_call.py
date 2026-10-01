"""A call, live (docs/DEVELOPER_GUIDE.md §5): put the caller on hold, take
them back, and end the call. The call's id comes from its events: the
``call_id`` of the ``call.initiated`` webhook delivery."""

import os
import sys
import time

from fcp_api.api.call_control import call_control_hangup, call_control_hold, call_control_resume
from fcp_api.models import HoldRequest
from fcp_sdk import client, ok


def run(url: str, token: str, call_id: str) -> list:
    fcp = client(url, token)

    # The caller hears the hold music; the other party no longer hears them.
    held = ok(call_control_hold.sync_detailed(call_id, client=fcp, body=HoldRequest(party="caller")))
    time.sleep(1)
    resumed = ok(call_control_resume.sync_detailed(call_id, client=fcp))

    # Elsewhere: transfer, redirect into a flow, play a prompt, say text —
    # each one POST to /api/v1/calls/{call_id}/<action> (docs/CALL_CONTROL.md).
    ended = ok(call_control_hangup.sync_detailed(call_id, client=fcp))
    return [held.status, resumed.status, ended.status]


if __name__ == "__main__":
    print(run(os.environ["FCP_URL"], os.environ["FCP_TOKEN"], sys.argv[1]))
