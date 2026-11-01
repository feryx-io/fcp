from enum import StrEnum

class DataClass(StrEnum):
    AUDIT_EVENT = "audit_event"
    CALL_RECORDING = "call_recording"
    CDR = "cdr"
    CONFERENCE_RECORDING = "conference_recording"
    DEVICE_CAPTURE = "device_capture"
    FLOW_TRACE = "flow_trace"
    SIP_TRACE = "sip_trace"
    VOICEMAIL = "voicemail"

    def __str__(self) -> str:
        return str(self.value)
