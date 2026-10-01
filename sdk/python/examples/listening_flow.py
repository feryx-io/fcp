"""A flow that listens (docs/DEVELOPER_GUIDE.md §4): a menu answered by
speech or digits, the caller's words posted to your service, and a number
that runs it."""

import os

from fcp_api.api.flows import flows_create_flow
from fcp_api.api.line_bindings import line_bindings_create_line_binding
from fcp_api.api.phone_numbers import phone_numbers_create_phone_number
from fcp_api.models import CreateFlowRequest, CreateLineBindingRequest, CreatePhoneNumberRequest
from fcp_sdk import client, ok


def run(
    url: str,
    token: str,
    flow_id: str = "front-desk",
    number: str = "+15125550170",
    crm: str = "https://crm.example.com",
) -> dict:
    fcp = client(url, token)

    flow = ok(
        flows_create_flow.sync_detailed(
            client=fcp,
            body=CreateFlowRequest(
                id=flow_id,
                name="Front desk",
                initial_node="answer",
                nodes={
                    "answer": {"type": "answer", "next": "menu"},
                    "menu": {
                        "type": "gather",
                        "input": "dtmf_speech",
                        "prompt": {"type": "say", "text": "Say sales or support, or press 1 or 2."},
                        "num_digits": 1,
                        "timeout": 5,
                        "hints": ["sales", "support"],
                        "transitions": {"1": "sales", "2": "support"},
                        "speech_transitions": {"sales": "sales", "support": "support"},
                        "on_timeout": "menu",
                    },
                    "sales": {
                        "type": "http",
                        "method": "POST",
                        "url": f"{crm}/leads",
                        "send_variables": True,  # caller, speech, speech_confidence, …
                        "response_variable": "crm",
                        "next": "to-sales",
                        "on_error": "to-sales",
                    },
                    "to-sales": {
                        "type": "dial",
                        "destination": "{{crm.owner_extension}}",
                        "next": "bye",
                        "on_error": "support",
                    },
                    "support": {"type": "queue", "queue_name": "support", "next_on_answer": "bye"},
                    "bye": {"type": "hangup"},
                },
            ),
        )
    )

    # A number whose calls run the flow.
    phone = ok(
        phone_numbers_create_phone_number.sync_detailed(
            client=fcp, body=CreatePhoneNumberRequest(e164=number)
        )
    )
    ok(
        line_bindings_create_line_binding.sync_detailed(
            client=fcp,
            body=CreateLineBindingRequest(phone_number_id=phone.id, binding_type="ivr", flow_id=flow.id),
        )
    )
    return {"flow": flow.id, "version": flow.version, "number": phone.e164}


if __name__ == "__main__":
    print(run(os.environ["FCP_URL"], os.environ["FCP_TOKEN"]))
