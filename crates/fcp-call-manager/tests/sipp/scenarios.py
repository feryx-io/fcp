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


def headers(extra):
    """Extra header lines, each on its own line of the message."""
    return "".join(f"\n      {line}" for line in extra.splitlines() if line)


def invite(cseq, auth, txn=None, extra=""):
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
      User-Agent: SIPp{headers(extra)}
{body(True)}
    ]]>
  </send>
"""


# The caller's INVITE, its challenge (401 or 407), the ACK to it, and the
# INVITE again with the digest of -au/-ap at CSeq 2; `txn` names that
# INVITE's transaction for a scenario that must tell its responses from a
# CANCEL's.
def uac_invite(txn=None, trying=True, extra=""):
    return (
        invite(1, False, extra=extra)
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
        + invite(2, True, txn, extra)
        + (
            f"""  <recv response="100" optional="true"{' response_txn="' + txn + '"' if txn else ""}/>
"""
            if trying
            else ""
        )
    )


UAC_INVITE = uac_invite()


def in_dialog(method, cseq, sdp=False, txn=None, extra=""):
    """A request inside the caller's dialog, to the callee's Contact.
    `txn` names the transaction it starts, or for an ACK the INVITE's."""
    retrans = "" if method == "ACK" else ' retrans="500"'
    if txn:
        retrans += f' {"ack_txn" if method == "ACK" else "start_txn"}="{txn}"'
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
      Max-Forwards: 70{headers(extra)}
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


def reply(code, reason, sdp=False, tag=True, cseq=None, via=None, extra="", next=None):
    """The callee's reply to the request it last received. `tag` adds its
    To tag (a dialog-forming reply); `cseq` overrides the CSeq and `via`
    the Via (both for a reply to an earlier request than the last). `next`
    jumps from the send itself: SIPp takes a message that arrives while its
    next step is not a receive for an unexpected one, so nothing may stand
    between a reply and the receives it goes back to."""
    to = "[last_To:];tag=[pid]SIPpTag01[call_number]" if tag else "[last_To:]"
    cs = f"CSeq: {cseq}" if cseq else "[last_CSeq:]"
    via = f"Via: {via}" if via else "[last_Via:]"
    jump = f' next="{next}"' if next else ""
    return f"""  <send{jump}>
    <![CDATA[
      SIP/2.0 {code} {reason}
      {via}
      [last_From:]
      {to}
      [last_Call-ID:]
      {cs}
      Contact: <sip:[local_ip]:[local_port];transport=[transport]>{headers(extra)}
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

# Mid-call requests (D2b): UPDATE, PRACK, REFER and session timers.

# The callee keeps its caller's identity, to send a request of its own in
# the dialog (a BYE): From and To as the INVITE had them, and its Contact.
REMEMBER_CALLER = """    <action>
      <ereg regexp=".*" search_in="hdr" header="From:" assign_to="caller"/>
      <ereg regexp=".*" search_in="hdr" header="To:" assign_to="callee"/>
      <ereg regexp="sip:[^>]*" search_in="hdr" header="Contact:" assign_to="contact"/>
    </action>
"""


def uas_bye(cseq=1):
    """The callee hangs up: a BYE to the INVITE's Contact."""
    return f"""  <send retrans="500">
    <![CDATA[
      BYE [$contact] SIP/2.0
      Via: SIP/2.0/[transport] [local_ip]:[local_port];branch=[branch]
      From: [$callee];tag=[pid]SIPpTag01[call_number]
      To: [$caller]
      Call-ID: [call_id]
      CSeq: {cseq} BYE
      Max-Forwards: 70
      Content-Length: 0

    ]]>
  </send>
  <recv response="200"/>
"""


# The callee answers and then takes whatever the node sends in the dialog
# (re-INVITEs, UPDATEs: session refreshes, a transfer's new media) until
# it is hung up on.
UAS_IN_DIALOG_UNTIL_BYE = (
    """  <label id="in_dialog"/>
  <recv request="INVITE" optional="true" next="reinvited"/>
  <recv request="UPDATE" optional="true" next="updated"/>
  <recv request="BYE" next="bye"/>
  <label id="reinvited"/>
"""
    + reply(200, "OK", sdp=True, tag=False)
    + """  <recv request="ACK" next="in_dialog"/>
  <label id="updated"/>
"""
    + reply(200, "OK", sdp=True, tag=False, next="in_dialog")
    + """  <label id="bye"/>
"""
    + reply(200, "OK", tag=False)
)

UAS_ANSWERED = (
    UAS_RING
    + reply(200, "OK", sdp=True)
    + """  <recv request="ACK" crlf="true"/>
"""
)

SCENARIOS.update(
    {
        "uac_update": (
            "A call answered, UPDATEd with a new offer (RFC 3311), then hung "
            "up. Keys: user, domain, callee.",
            UAC_INVITE
            + """  <recv response="180" optional="true"/>
  <recv response="183" optional="true"/>
  <recv response="200" rrs="true"/>
"""
            + in_dialog("ACK", 2)
            + """  <pause milliseconds="300"/>
"""
            + in_dialog("UPDATE", 3, sdp=True)
            + """  <recv response="200">
    <action>
      <ereg regexp="m=audio" search_in="body" check_it="true" assign_to="answer"/>
      <log message="answer: [$answer]"/>
    </action>
  </recv>
  <pause milliseconds="300"/>
"""
            + in_dialog("BYE", 4)
            + """  <recv response="200"/>
""",
        ),
        "uas_until_bye": (
            "The callee: answers, takes re-INVITEs and UPDATEs, and is hung "
            "up on.",
            UAS_ANSWERED + UAS_IN_DIALOG_UNTIL_BYE,
        ),
        "uac_prack": (
            "A caller supporting 100rel (RFC 3262) called by a callee that "
            "rings reliably: the node PRACKs the callee itself, and the 180 "
            "reaches the caller as an ordinary one (no RSeq, no Require: "
            "100rel). Keys: user, domain, callee.",
            uac_invite(extra="Supported: 100rel")
            + """  <recv response="183" optional="true"/>
  <recv response="180">
    <action>
      <ereg regexp="[0-9]" search_in="hdr" header="RSeq:" check_it_inverse="true" assign_to="rseq"/>
      <log message="rseq: [$rseq]"/>
    </action>
  </recv>
  <recv response="200" rrs="true"/>
"""
            + in_dialog("ACK", 2)
            + """  <pause milliseconds="300"/>
"""
            + in_dialog("BYE", 3)
            + """  <recv response="200"/>
""",
        ),
        "uas_prack": (
            "The callee: offered 100rel, rings reliably (Require: 100rel, "
            "RSeq 1), waits for the PRACK, answers it, then answers the INVITE "
            "and is hung up on.",
            """  <recv request="INVITE" crlf="true">
    <action>
      <ereg regexp=".*" search_in="hdr" header="Via:" assign_to="invite_via"/>
      <ereg regexp="[0-9]+" search_in="hdr" header="CSeq:" assign_to="invite_cseq"/>
      <ereg regexp="100rel" search_in="hdr" header="Supported:" check_it="true" assign_to="supported"/>
      <log message="supported: [$supported]"/>
    </action>
  </recv>
"""
            + reply(180, "Ringing", extra="Require: 100rel\nRSeq: 1")
            + """  <recv request="PRACK" crlf="true">
    <action>
      <ereg regexp="1 [0-9]+ INVITE" search_in="hdr" header="RAck:" check_it="true" assign_to="rack"/>
      <log message="rack: [$rack]"/>
    </action>
  </recv>
"""
            + reply(200, "OK", tag=False)
            + """  <pause milliseconds="200"/>
"""
            + reply(
                200, "OK", sdp=True, tag=False, cseq="[$invite_cseq] INVITE", via="[$invite_via]"
            )
            + """  <recv request="ACK" crlf="true"/>
"""
            + UAS_IN_DIALOG_UNTIL_BYE,
        ),
        "uac_refer": (
            "A call answered, then the caller transfers the callee blind "
            "(REFER, RFC 3515) to [target]: accepted, the outcome NOTIFYed, "
            "and the caller hung up on by the node. Keys: user, domain, "
            "callee, target.",
            UAC_INVITE
            + """  <recv response="180" optional="true"/>
  <recv response="183" optional="true"/>
  <recv response="200" rrs="true"/>
"""
            + in_dialog("ACK", 2)
            + """  <pause milliseconds="300"/>
"""
            + in_dialog(
                "REFER",
                3,
                txn="refer",
                extra="Refer-To: <sip:[target]@[domain]>\nReferred-By: <sip:[user]@[domain]>",
            )
            + """  <recv response="202" response_txn="refer"/>
  <label id="waiting"/>
  <recv request="NOTIFY" optional="true" next="notified"/>
  <recv request="BYE" next="bye"/>
  <label id="notified"/>
"""
            + reply(200, "OK", tag=False, next="waiting")
            + """  <label id="bye"/>
"""
            + reply(200, "OK", tag=False),
        ),
        "uas_answer_then_bye": (
            "The callee: answers, and hangs up a second later.",
            """  <recv request="INVITE" crlf="true">
"""
            + REMEMBER_CALLER
            + """  </recv>
"""
            + reply(180, "Ringing")
            + reply(200, "OK", sdp=True)
            + """  <recv request="ACK" crlf="true"/>
  <pause milliseconds="1000"/>
"""
            + uas_bye(),
        ),
        "uac_session_too_short": (
            "An INVITE asking for a 30 s session (RFC 4028): refused 422 with "
            "the node's Min-SE, before it is challenged. Keys: user, domain, "
            "callee.",
            invite(1, False, extra="Supported: timer\nSession-Expires: 30")
            + """  <recv response="100" optional="true"/>
  <recv response="422">
    <action>
      <ereg regexp="[0-9]+" search_in="hdr" header="Min-SE:" check_it="true" assign_to="min_se"/>
      <log message="min_se: [$min_se]"/>
    </action>
  </recv>
"""
            + raw("""      ACK sip:[callee]@[domain] SIP/2.0
      [last_Via:]
      From: <sip:[user]@[domain]>;tag=[pid]SIPpTag00[call_number]
      [last_To:]
      Call-ID: [call_id]
      CSeq: 1 ACK
      Max-Forwards: 70
      Content-Length: 0
""")
            + """""",
        ),
    }
)


def uac_session(refresher, refreshes):
    """A call asking for a 90 s session with `refresher` refreshing it. The
    answer must keep the request's interval and refresher (RFC 4028 §9).
    `refreshes`: the node refreshes and the caller answers, then hangs up;
    otherwise the caller never refreshes, and the node must end the call
    before the session expires."""
    answer_checks = f"""    <action>
      <ereg regexp="^ *90 *;.*refresher=?{refresher}" search_in="hdr" header="Session-Expires:" check_it="true" assign_to="se"/>
      <log message="se: [$se]"/>
    </action>
"""
    head = (
        uac_invite(extra=f"Supported: timer\nSession-Expires: 90;refresher={refresher}")
        + f"""  <recv response="180" optional="true"/>
  <recv response="183" optional="true"/>
  <recv response="200" rrs="true">
{answer_checks}  </recv>
"""
        + in_dialog("ACK", 2)
    )
    if refreshes:
        tail = (
            """  <recv request="UPDATE" optional="true" next="updated"/>
  <recv request="INVITE" timeout="70000" next="reinvited"/>
  <label id="updated"/>
"""
            + reply(200, "OK", tag=False, extra="Supported: timer\nSession-Expires: 90;refresher=uas", next="refreshed")
            + """  <label id="reinvited"/>
"""
            + reply(200, "OK", sdp=True, tag=False, extra="Supported: timer\nSession-Expires: 90;refresher=uas")
            + """  <recv request="ACK"/>
  <label id="refreshed"/>
"""
            + in_dialog("BYE", 3)
            + """  <recv response="200"/>
"""
        )
    else:
        tail = (
            """  <recv request="BYE" timeout="75000"/>
"""
            + reply(200, "OK", tag=False)
        )
    return head + tail


SCENARIOS.update(
    {
        "uac_session_refreshed": (
            "A call asking for a 90 s session refreshed by the node "
            "(refresher=uas): the node refreshes it within the interval. "
            "Keys: user, domain, callee.",
            uac_session("uas", True),
        ),
        "uac_session_expires": (
            "A call asking for a 90 s session refreshed by the caller "
            "(refresher=uac), which never refreshes: the node ends it before "
            "it expires. Keys: user, domain, callee.",
            uac_session("uac", False),
        ),
    }
)

RACE_DELAYS = (0, 50)
SCENARIOS.update({f"uas_answer_after_{ms}": uas_answer_after(ms) for ms in RACE_DELAYS})

if __name__ == "__main__":
    for name, (doc, steps) in SCENARIOS.items():
        (HERE / f"{name}.xml").write_text(scenario(name, doc, steps))
        print(f"wrote {name}.xml")
