from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="UpdatePolicyRequest")



@_attrs_define
class UpdatePolicyRequest:
    """ Every field optional; `null` on a retention field means "keep forever".

        Attributes:
            cdr_retention_days (int | None | Unset):
            sip_trace_retention_days (int | None | Unset):
            flow_trace_retention_days (int | None | Unset):
            recording_retention_days (int | None | Unset):
            audit_retention_days (int | None | Unset):
            voicemail_retention_days (int | None | Unset):
            recording_consent (None | str | Unset):
            recording_consent_prompt_id (None | str | Unset):
            recording_consent_on_failure (None | str | Unset):
            recording_notice_interval_secs (int | None | Unset):
            subject_request_days (int | None | Unset):
            cdr_erasure (None | str | Unset):
            cdr_archive_required (bool | None | Unset):
     """

    cdr_retention_days: int | None | Unset = UNSET
    sip_trace_retention_days: int | None | Unset = UNSET
    flow_trace_retention_days: int | None | Unset = UNSET
    recording_retention_days: int | None | Unset = UNSET
    audit_retention_days: int | None | Unset = UNSET
    voicemail_retention_days: int | None | Unset = UNSET
    recording_consent: None | str | Unset = UNSET
    recording_consent_prompt_id: None | str | Unset = UNSET
    recording_consent_on_failure: None | str | Unset = UNSET
    recording_notice_interval_secs: int | None | Unset = UNSET
    subject_request_days: int | None | Unset = UNSET
    cdr_erasure: None | str | Unset = UNSET
    cdr_archive_required: bool | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        cdr_retention_days: int | None | Unset
        if isinstance(self.cdr_retention_days, Unset):
            cdr_retention_days = UNSET
        else:
            cdr_retention_days = self.cdr_retention_days

        sip_trace_retention_days: int | None | Unset
        if isinstance(self.sip_trace_retention_days, Unset):
            sip_trace_retention_days = UNSET
        else:
            sip_trace_retention_days = self.sip_trace_retention_days

        flow_trace_retention_days: int | None | Unset
        if isinstance(self.flow_trace_retention_days, Unset):
            flow_trace_retention_days = UNSET
        else:
            flow_trace_retention_days = self.flow_trace_retention_days

        recording_retention_days: int | None | Unset
        if isinstance(self.recording_retention_days, Unset):
            recording_retention_days = UNSET
        else:
            recording_retention_days = self.recording_retention_days

        audit_retention_days: int | None | Unset
        if isinstance(self.audit_retention_days, Unset):
            audit_retention_days = UNSET
        else:
            audit_retention_days = self.audit_retention_days

        voicemail_retention_days: int | None | Unset
        if isinstance(self.voicemail_retention_days, Unset):
            voicemail_retention_days = UNSET
        else:
            voicemail_retention_days = self.voicemail_retention_days

        recording_consent: None | str | Unset
        if isinstance(self.recording_consent, Unset):
            recording_consent = UNSET
        else:
            recording_consent = self.recording_consent

        recording_consent_prompt_id: None | str | Unset
        if isinstance(self.recording_consent_prompt_id, Unset):
            recording_consent_prompt_id = UNSET
        else:
            recording_consent_prompt_id = self.recording_consent_prompt_id

        recording_consent_on_failure: None | str | Unset
        if isinstance(self.recording_consent_on_failure, Unset):
            recording_consent_on_failure = UNSET
        else:
            recording_consent_on_failure = self.recording_consent_on_failure

        recording_notice_interval_secs: int | None | Unset
        if isinstance(self.recording_notice_interval_secs, Unset):
            recording_notice_interval_secs = UNSET
        else:
            recording_notice_interval_secs = self.recording_notice_interval_secs

        subject_request_days: int | None | Unset
        if isinstance(self.subject_request_days, Unset):
            subject_request_days = UNSET
        else:
            subject_request_days = self.subject_request_days

        cdr_erasure: None | str | Unset
        if isinstance(self.cdr_erasure, Unset):
            cdr_erasure = UNSET
        else:
            cdr_erasure = self.cdr_erasure

        cdr_archive_required: bool | None | Unset
        if isinstance(self.cdr_archive_required, Unset):
            cdr_archive_required = UNSET
        else:
            cdr_archive_required = self.cdr_archive_required


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if cdr_retention_days is not UNSET:
            field_dict["cdr_retention_days"] = cdr_retention_days
        if sip_trace_retention_days is not UNSET:
            field_dict["sip_trace_retention_days"] = sip_trace_retention_days
        if flow_trace_retention_days is not UNSET:
            field_dict["flow_trace_retention_days"] = flow_trace_retention_days
        if recording_retention_days is not UNSET:
            field_dict["recording_retention_days"] = recording_retention_days
        if audit_retention_days is not UNSET:
            field_dict["audit_retention_days"] = audit_retention_days
        if voicemail_retention_days is not UNSET:
            field_dict["voicemail_retention_days"] = voicemail_retention_days
        if recording_consent is not UNSET:
            field_dict["recording_consent"] = recording_consent
        if recording_consent_prompt_id is not UNSET:
            field_dict["recording_consent_prompt_id"] = recording_consent_prompt_id
        if recording_consent_on_failure is not UNSET:
            field_dict["recording_consent_on_failure"] = recording_consent_on_failure
        if recording_notice_interval_secs is not UNSET:
            field_dict["recording_notice_interval_secs"] = recording_notice_interval_secs
        if subject_request_days is not UNSET:
            field_dict["subject_request_days"] = subject_request_days
        if cdr_erasure is not UNSET:
            field_dict["cdr_erasure"] = cdr_erasure
        if cdr_archive_required is not UNSET:
            field_dict["cdr_archive_required"] = cdr_archive_required

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_cdr_retention_days(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        cdr_retention_days = _parse_cdr_retention_days(d.pop("cdr_retention_days", UNSET))


        def _parse_sip_trace_retention_days(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        sip_trace_retention_days = _parse_sip_trace_retention_days(d.pop("sip_trace_retention_days", UNSET))


        def _parse_flow_trace_retention_days(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        flow_trace_retention_days = _parse_flow_trace_retention_days(d.pop("flow_trace_retention_days", UNSET))


        def _parse_recording_retention_days(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        recording_retention_days = _parse_recording_retention_days(d.pop("recording_retention_days", UNSET))


        def _parse_audit_retention_days(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        audit_retention_days = _parse_audit_retention_days(d.pop("audit_retention_days", UNSET))


        def _parse_voicemail_retention_days(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        voicemail_retention_days = _parse_voicemail_retention_days(d.pop("voicemail_retention_days", UNSET))


        def _parse_recording_consent(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        recording_consent = _parse_recording_consent(d.pop("recording_consent", UNSET))


        def _parse_recording_consent_prompt_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        recording_consent_prompt_id = _parse_recording_consent_prompt_id(d.pop("recording_consent_prompt_id", UNSET))


        def _parse_recording_consent_on_failure(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        recording_consent_on_failure = _parse_recording_consent_on_failure(d.pop("recording_consent_on_failure", UNSET))


        def _parse_recording_notice_interval_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        recording_notice_interval_secs = _parse_recording_notice_interval_secs(d.pop("recording_notice_interval_secs", UNSET))


        def _parse_subject_request_days(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        subject_request_days = _parse_subject_request_days(d.pop("subject_request_days", UNSET))


        def _parse_cdr_erasure(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        cdr_erasure = _parse_cdr_erasure(d.pop("cdr_erasure", UNSET))


        def _parse_cdr_archive_required(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        cdr_archive_required = _parse_cdr_archive_required(d.pop("cdr_archive_required", UNSET))


        update_policy_request = cls(
            cdr_retention_days=cdr_retention_days,
            sip_trace_retention_days=sip_trace_retention_days,
            flow_trace_retention_days=flow_trace_retention_days,
            recording_retention_days=recording_retention_days,
            audit_retention_days=audit_retention_days,
            voicemail_retention_days=voicemail_retention_days,
            recording_consent=recording_consent,
            recording_consent_prompt_id=recording_consent_prompt_id,
            recording_consent_on_failure=recording_consent_on_failure,
            recording_notice_interval_secs=recording_notice_interval_secs,
            subject_request_days=subject_request_days,
            cdr_erasure=cdr_erasure,
            cdr_archive_required=cdr_archive_required,
        )


        update_policy_request.additional_properties = d
        return update_policy_request

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
