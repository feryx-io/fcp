"""The webhook vector every FCP implementation checks against
(sdk/typescript/test/webhook.test.js, and the dispatcher's own tests)."""

from fcp_sdk import verify_webhook

SECRET = "whsec_test_vector"
T = 1790000000
BODY = '{"event":"call.started"}'
V1 = "008976dcc4d8a2d92dbfe2c643bdb8f498825fa20a43baa8d0e7a0358ffa159e"


def test_a_genuine_delivery_verifies():
    assert verify_webhook(SECRET, f"t={T},v1={V1}", BODY, now=T + 60)
    assert verify_webhook(SECRET, f"t={T},v1={V1}", BODY.encode(), now=T)


def test_a_stale_delivery_does_not_however_genuine():
    assert not verify_webhook(SECRET, f"t={T},v1={V1}", BODY, now=T + 301)
    assert verify_webhook(SECRET, f"t={T},v1={V1}", BODY, now=T + 900, tolerance_secs=1000)


def test_a_changed_body_a_wrong_secret_or_a_bad_header_does_not():
    assert not verify_webhook(SECRET, f"t={T},v1={V1}", BODY + " ", now=T)
    assert not verify_webhook("whsec_other", f"t={T},v1={V1}", BODY, now=T)
    assert not verify_webhook(SECRET, f"v1={V1}", BODY, now=T)
    assert not verify_webhook(SECRET, f"t={T},v1=zz", BODY, now=T)


def test_while_a_secret_rotates_either_signature_does():
    assert verify_webhook(SECRET, f"t={T},v1={'0' * 64},v1={V1}", BODY, now=T)
