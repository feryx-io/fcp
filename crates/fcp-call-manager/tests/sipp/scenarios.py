#!/usr/bin/env python3
"""Writes the call scenarios in this directory (docs/DEEP_TESTING.md D2).

SIPp scenarios are XML and mostly the same text: an INVITE, its digest
challenge, an in-dialog request, a reply. This builds them from those parts
so a change to one (the SDP, a header) is made once. Run it after an edit
and commit what it writes; tests/sipp.rs runs the XML. The registration
scenarios are written by hand.
"""

from pathlib import Path

HERE = Path(__file__).resolve().parent

SDP = """v=0
      o=sipp 1 1 IN IP[local_ip_type] [local_ip]
      s=-
      c=IN IP[media_ip_type] [media_ip]
      t=0 0
      m=audio [media_port] RTP/AVP 0 101
      a=rtpmap:0 PCMU/8000
      a=rtpmap:101 telephone-event/8000
      a=sendrecv"""

HEAD = """<?xml version="1.0" encoding="ISO-8859-1" ?>
<!DOCTYPE scenario SYSTEM "sipp.dtd">
"""


def body(sdp):
    if sdp:
        return f"""      Content-Type: application/sdp
      Content-Length: [len]

      {SDP}"""
    return """      Content-Length: 0
"""


AUTH = """
      [authentication]"""


def invite(cseq, auth, txn=None):
    start = f' start_txn="{txn}"' if txn else ""
    return f"""  <send retrans="500"{start}>
    <![CDATA[
      INVITE sip:[callee]@[domain] SIP/2.0
      Via: SIP/2.0/[transport] [local_ip]:[local_port];branch=[branch]
      From: <sip:[user]@[domain]>;tag=[pid]SIPpTag00[call_number]
      To: <sip:[callee]@[domain]>
      Call-ID: [call_id]
      CSeq: {cseq} INVITE
      Contact: <sip:[user]@[local_ip]:[local_port]>{AUTH if auth else ""}
      Max-Forwards: 70
      User-Agent: SIPp
{body(True)}
    ]]>
  </send>
"""


# The caller's INVITE, its challenge (401 or 407), the ACK to it, and the
# INVITE again with the digest of -au/-ap at CSeq 2; `txn` names that
# INVITE's transaction for a scenario that must tell its responses from a
# CANCEL's.
def uac_invite(txn=None, trying=True):
    return (
        invite(1, False)
        + """  <recv response="100" optional="true"/>
  <recv response="407" optional="true" auth="true" next="challenged"/>
  <recv response="401" auth="true"/>
  <label id="challenged"/>
  <send>
    <![CDATA[
      ACK sip:[callee]@[domain] SIP/2.0
      [last_Via:]
      From: <sip:[user]@[domain]>;tag=[pid]SIPpTag00[call_number]
      [last_To:]
      Call-ID: [call_id]
      CSeq: 1 ACK
      Max-Forwards: 70
      Content-Length: 0

    ]]>
  </send>
"""
        + invite(2, True, txn)
        + (
            f"""  <recv response="100" optional="true"{' response_txn="' + txn + '"' if txn else ""}/>
"""
            if trying
            else ""
        )
    )


UAC_INVITE = uac_invite()


def in_dialog(method, cseq, sdp=False, txn=None):
    """A request inside the caller's dialog, to the callee's Contact."""
    retrans = "" if method == "ACK" else ' retrans="500"'
    if txn:
        retrans += f' start_txn="{txn}"'
    return f"""  <send{retrans}>
    <![CDATA[
      {method} [next_url] SIP/2.0
      Via: SIP/2.0/[transport] [local_ip]:[local_port];branch=[branch]
      From: <sip:[user]@[domain]>;tag=[pid]SIPpTag00[call_number]
      To: <sip:[callee]@[domain]>[peer_tag_param]
      Call-ID: [call_id]
      CSeq: {cseq} {method}
      Contact: <sip:[user]@[local_ip]:[local_port]>
      [routes]
      Max-Forwards: 70
{body(sdp)}
    ]]>
  </send>
"""


def cancel():
    """CANCEL of the caller's authenticated INVITE (CSeq 2), on its branch."""
    return """  <send retrans="500" start_txn="cancel">
    <![CDATA[
      CANCEL sip:[callee]@[domain] SIP/2.0
      [last_Via:]
      From: <sip:[user]@[domain]>;tag=[pid]SIPpTag00[call_number]
      To: <sip:[callee]@[domain]>
      Call-ID: [call_id]
      CSeq: 2 CANCEL
      Max-Forwards: 70
      Content-Length: 0

    ]]>
  </send>
"""


def ack_failure(txn=None):
    """ACK to a final failure of the INVITE at CSeq 2 (of transaction
    `txn` when it is named)."""
    ack = f' ack_txn="{txn}"' if txn else ""
    return f"""  <send{ack}>
    <![CDATA[
      ACK sip:[callee]@[domain] SIP/2.0
      [last_Via:]
      From: <sip:[user]@[domain]>;tag=[pid]SIPpTag00[call_number]
      [last_To:]
      Call-ID: [call_id]
      CSeq: 2 ACK
      Max-Forwards: 70
      Content-Length: 0

    ]]>
  </send>
"""


def reply(code, reason, sdp=False, tag=True, cseq=None):
    """The callee's reply to the request it last received. `tag` adds its
    To tag (a dialog-forming reply); `cseq` overrides the CSeq."""
    to = "[last_To:];tag=[pid]SIPpTag01[call_number]" if tag else "[last_To:]"
    cs = f"CSeq: {cseq}" if cseq else "[last_CSeq:]"
    return f"""  <send>
    <![CDATA[
      SIP/2.0 {code} {reason}
      [last_Via:]
      [last_From:]
      {to}
      [last_Call-ID:]
      {cs}
      Contact: <sip:[local_ip]:[local_port];transport=[transport]>
{body(sdp)}
    ]]>
  </send>
"""


def scenario(name, doc, steps):
    return f"""{HEAD}<!-- {doc} -->
<scenario name="{name}">
{steps}</scenario>
"""


SCENARIOS = {
    "uac_answered": (
        "A call answered, then hung up by the caller. Keys: user, domain, callee.",
        UAC_INVITE
        + """  <recv response="180" optional="true"/>
  <recv response="183" optional="true"/>
  <recv response="200" rrs="true"/>
"""
        + in_dialog("ACK", 2)
        + """  <pause milliseconds="500"/>
"""
        + in_dialog("BYE", 3)
        + """  <recv response="200"/>
""",
    ),
    "uas_answer": (
        "The callee: rings, answers, and is hung up on.",
        """  <recv request="INVITE" crlf="true"/>
"""
        + reply(180, "Ringing")
        + reply(200, "OK", sdp=True)
        + """  <recv request="ACK" crlf="true"/>
  <recv request="BYE"/>
"""
        + reply(200, "OK", tag=False),
    ),
}

UAS_RING = """  <recv request="INVITE" crlf="true"/>
""" + reply(180, "Ringing")

ANSWERED_THEN_BYE = (
    """  <recv response="200" rrs="true"/>
"""
    + in_dialog("ACK", 2)
    + """  <pause milliseconds="300"/>
"""
    + in_dialog("BYE", 3)
    + """  <recv response="200"/>
"""
)

SCENARIOS.update(
    {
        "uac_busy": (
            "A call the callee refuses busy. Keys: user, domain, callee.",
            UAC_INVITE
            + """  <recv response="180" optional="true"/>
  <recv response="486"/>
"""
            + ack_failure(),
        ),
        "uas_busy": (
            "The callee: busy.",
            """  <recv request="INVITE" crlf="true"/>
"""
            + reply(486, "Busy Here")
            + """  <recv request="ACK"/>
""",
        ),
        "uac_cancel": (
            "A call the caller cancels while it rings. Keys: user, domain, callee.",
            uac_invite("invite2")
            + """  <recv response="183" optional="true"/>
  <recv response="180"/>
  <pause milliseconds="300"/>
"""
            + cancel()
            + """  <recv response="200" response_txn="cancel"/>
  <recv response="487" response_txn="invite2"/>
"""
            + ack_failure("invite2"),
        ),
        "uas_ring": (
            "The callee: rings until it is cancelled.",
            UAS_RING
            + """  <recv request="CANCEL"/>
"""
            + reply(200, "OK")
            + reply(487, "Request Terminated", cseq="[last_cseq_number] INVITE")
            + """  <recv request="ACK"/>
""",
        ),
        "uac_cancel_race": (
            "The caller cancels on the callee's 180 while the callee answers: "
            "either the call is cancelled (487) or answered (200, then hung "
            "up). Keys: user, domain, callee.",
            uac_invite("invite2")
            + """  <recv response="183" optional="true" response_txn="invite2"/>
  <recv response="180" response_txn="invite2"/>
"""
            + cancel()
            + """  <recv response="200" response_txn="cancel" optional="true"/>
  <recv response="481" response_txn="cancel" optional="true"/>
  <recv response="487" response_txn="invite2" optional="true" next="cancelled"/>
  <recv response="200" response_txn="invite2" rrs="true"/>
"""
            + in_dialog("ACK", 2)
            + in_dialog("BYE", 3, txn="bye")
            + """  <recv response="200" response_txn="cancel" optional="true"/>
  <recv response="481" response_txn="cancel" optional="true"/>
  <recv response="200" response_txn="bye"/>
  <pause milliseconds="200" next="done"/>
  <label id="cancelled"/>
"""
            + ack_failure("invite2")
            + """  <label id="done"/>
""",
        ),
        "uac_cancel_early": (
            "The caller cancels on the node's 100, before the call is set up: "
            "it is cancelled (487), never answered. Keys: user, domain, callee.",
            uac_invite("invite2", trying=False)
            + """  <recv response="100" response_txn="invite2"/>
"""
            + cancel()
            + """  <recv response="200" response_txn="cancel"/>
  <recv response="487" response_txn="invite2"/>
"""
            + ack_failure("invite2"),
        ),
        "uac_reinvite": (
            "A call answered, re-INVITEd with a new offer, then hung up. "
            "Keys: user, domain, callee.",
            UAC_INVITE
            + """  <recv response="180" optional="true"/>
  <recv response="183" optional="true"/>
  <recv response="200" rrs="true"/>
"""
            + in_dialog("ACK", 2)
            + """  <pause milliseconds="300"/>
"""
            + in_dialog("INVITE", 3, sdp=True)
            + """  <recv response="100" optional="true"/>
  <recv response="200"/>
"""
            + in_dialog("ACK", 3)
            + """  <pause milliseconds="300"/>
"""
            + in_dialog("BYE", 4)
            + """  <recv response="200"/>
""",
        ),
        "uas_reinvite": (
            "The callee: answers, takes a re-INVITE if one reaches it, and "
            "is hung up on.",
            UAS_RING
            + reply(200, "OK", sdp=True)
            + """  <recv request="ACK" crlf="true"/>
  <recv request="INVITE" optional="true" next="reinvited"/>
  <recv request="BYE" next="bye"/>
  <label id="reinvited"/>
"""
            + reply(200, "OK", sdp=True, tag=False)
            + """  <recv request="ACK"/>
  <recv request="BYE"/>
  <label id="bye"/>
"""
            + reply(200, "OK", tag=False),
        ),
    }
)


def uas_answer_after(ms):
    """The callee for the CANCEL race: rings, and answers `ms` after
    ringing unless a CANCEL comes first. Cancelled first, it ends the call
    with 487; answered first, it is then cancelled or ACKed and, either
    way, must be sent a BYE by the node."""
    wait = (
        f"""  <recv request="CANCEL" timeout="{ms}" ontimeout="answer"/>
"""
        + reply(200, "OK")
        + reply(487, "Request Terminated", cseq="[last_cseq_number] INVITE")
        + """  <recv request="ACK" next="done"/>
  <label id="answer"/>
"""
        if ms
        else ""
    )
    return (
        f"The callee for the CANCEL race: answers {ms} ms after ringing unless "
        "cancelled first; answered, the node must end its leg with a BYE.",
        UAS_RING
        + wait
        + reply(200, "OK", sdp=True)
        + """  <recv request="CANCEL" optional="true" next="cancelled"/>
  <recv request="ACK"/>
  <recv request="BYE" next="bye"/>
  <label id="cancelled"/>
"""
        + reply(200, "OK")
        + """  <recv request="ACK"/>
  <recv request="BYE"/>
  <label id="bye"/>
"""
        + reply(200, "OK", tag=False)
        + """  <label id="done"/>
""",
    )


# 0: the 200 right behind the 180, crossing the CANCEL at the node. 50: the
# CANCEL first.


def stray(method, cseq, to_tag=True, sdp=False):
    """A request inside a dialog that does not exist."""
    tag = ";tag=no-such-dialog" if to_tag else ""
    retrans = "" if method == "ACK" else ' retrans="500"'
    return f"""  <send{retrans}>
    <![CDATA[
      {method} sip:[callee]@[remote_ip]:[remote_port] SIP/2.0
      Via: SIP/2.0/[transport] [local_ip]:[local_port];branch=[branch]
      From: <sip:[user]@[domain]>;tag=[pid]SIPpTag00[call_number]
      To: <sip:[callee]@[domain]>{tag}
      Call-ID: [call_id]
      CSeq: {cseq} {method}
      Contact: <sip:[user]@[local_ip]:[local_port]>
      Max-Forwards: 70
{body(sdp)}
    ]]>
  </send>
"""


def raw(text):
    """Bytes sent as they are, for a message no parser should accept."""
    return f"""  <send>
    <![CDATA[
{text}
    ]]>
  </send>
"""


SCENARIOS["strays"] = (
    "Requests inside dialogs that do not exist: each answered 481 (RFC 3261 "
    "§12.2.2), a re-INVITE 481 or challenged as a new call, a CANCEL for "
    "nothing 481 or 200, an ACK for nothing not at all. Keys: user, domain, "
    "callee.",
    stray("BYE", 1)
    + """  <recv response="481"/>
"""
    + stray("INFO", 2)
    + """  <recv response="481"/>
"""
    + stray("UPDATE", 3, sdp=True)
    + """  <recv response="481"/>
"""
    # A re-INVITE for a dialog the node does not have may be refused 481 or
    # taken as a new call (RFC 3261 §12.2.2), here challenged as one; it is
    # never connected unasked.
    + stray("INVITE", 4, sdp=True)
    + """  <recv response="100" optional="true"/>
  <recv response="481" optional="true" next="refused"/>
  <recv response="407" optional="true" next="refused"/>
  <recv response="401"/>
  <label id="refused"/>
  <send>
    <![CDATA[
      ACK sip:[callee]@[remote_ip]:[remote_port] SIP/2.0
      [last_Via:]
      From: <sip:[user]@[domain]>;tag=[pid]SIPpTag00[call_number]
      [last_To:]
      Call-ID: [call_id]
      CSeq: 4 ACK
      Max-Forwards: 70
      Content-Length: 0

    ]]>
  </send>
"""
    + stray("ACK", 7)
    + stray("CANCEL", 5, to_tag=False)
    + """  <recv response="481" optional="true" next="after_cancel"/>
  <recv response="200"/>
  <label id="after_cancel"/>
"""
    + stray("OPTIONS", 6, to_tag=False)
    + """  <recv response="200"/>
""",
)

def junk(n, first_line, headers, body=""):
    """A broken message on a Call-ID of its own, so whatever the node
    answers is not taken for this scenario's."""
    lines = [first_line] + headers + [f"Call-ID: junk-{n}-[pid]@sipp", ""]
    text = "\n".join("      " + l if l else "" for l in lines)
    return raw(text + ("\n      " + body if body else "\n"))


VIA = "Via: SIP/2.0/[transport] [local_ip]:[local_port];branch=[branch]"
FROM = "From: <sip:[user]@[domain]>;tag=junk"
TO = "To: <sip:[callee]@[domain]>"

SCENARIOS["junk"] = (
    "Messages no parser should accept, and a stray response, each on its own "
    "Call-ID; then an OPTIONS, which must still be answered. Keys: user, "
    "domain, callee.",
    junk(1, "THIS IS NOT SIP", [])
    + junk(2, "INVITE not-a-uri SIP/2.0", [FROM, TO, "CSeq: 1 INVITE", "Content-Length: 0"])
    + junk(3, "OPTIONS sip:[domain] SIP/2.0", [VIA, FROM, TO, "CSeq: 1 OPTIONS", "A header without a colon", "Content-Length: 0"])
    + junk(4, "OPTIONS sip:[domain] SIP/2.0", [VIA, FROM, TO, "CSeq: 1 OPTIONS", "Content-Length: 100000"], "short")
    + junk(5, "OPTIONS sip:[domain] SIP/2.0", [VIA, FROM, TO, "CSeq: 1 OPTIONS", "Content-Length: -5"])
    + junk(6, "OPTIONS sip:[domain] SIP/2.0", [VIA] * 70 + [FROM, TO, "CSeq: 1 OPTIONS", "Content-Length: 0"])
    + junk(7, "OPTIONS sip:[domain] SIP/2.0", [VIA, FROM, TO, "CSeq: 1 OPTIONS", "X-Pad: " + "a" * 30000, "Content-Length: 0"])
    + junk(8, "SIP/2.0 200 OK", [VIA, FROM, TO + ";tag=x", "CSeq: 1 INVITE", "Content-Length: 0"])
    + junk(9, "INVITE sip:[callee]@[domain] SIP/2.0", [VIA, FROM, TO, "CSeq: banana INVITE", "Content-Length: 0"])
    + """  <pause milliseconds="300"/>
"""
    + stray("OPTIONS", 1, to_tag=False)
    + """  <recv response="200"/>
""",
)

RACE_DELAYS = (0, 50)
SCENARIOS.update({f"uas_answer_after_{ms}": uas_answer_after(ms) for ms in RACE_DELAYS})

if __name__ == "__main__":
    for name, (doc, steps) in SCENARIOS.items():
        (HERE / f"{name}.xml").write_text(scenario(name, doc, steps))
        print(f"wrote {name}.xml")
