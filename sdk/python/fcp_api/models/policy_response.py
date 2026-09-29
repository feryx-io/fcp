from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="PolicyResponse")



@_attrs_define
class PolicyResponse:
    """ 
        Attributes:
            tenant_id (str):
            recording_consent (str):
            recording_consent_on_failure (str): `record` or `skip`: whether a recording goes ahead when its
                announcement cannot be played.
            subject_request_days (int): Days the tenant gives itself to answer a subject request.
            cdr_erasure (str): `pseudonymise` or `delete`: what an erasure does to call records.
            cdr_archive_required (bool): CDRs are deleted only once the archive has confirmed them.
            configured (bool): False until the tenant has saved a policy (defaults shown).
            updated_at (datetime.datetime):
            cdr_retention_days (int | None | Unset):
            sip_trace_retention_days (int | None | Unset):
            flow_trace_retention_days (int | None | Unset):
            recording_retention_days (int | None | Unset):
            audit_retention_days (int | None | Unset):
            voicemail_retention_days (int | None | Unset):
            recording_consent_prompt_id (None | str | Unset):
            recording_notice_interval_secs (int | None | Unset): A notice tone every this many seconds while a call is
                recorded.
            cdr_archived_before (int | None | Unset): Unix seconds: every one of the tenant's CDRs that started before
                this is in the archive, confirmed. `null` when none is.
     """

    tenant_id: str
    recording_consent: str
    recording_consent_on_failure: str
    subject_request_days: int
    cdr_erasure: str
    cdr_archive_required: bool
    configured: bool
    updated_at: datetime.datetime
    cdr_retention_days: int | None | Unset = UNSET
    sip_trace_retention_days: int | None | Unset = UNSET
    flow_trace_retention_days: int | None | Unset = UNSET
    recording_retention_days: int | None | Unset = UNSET
    audit_retention_days: int | None | Unset = UNSET
    voicemail_retention_days: int | None | Unset = UNSET
    recording_consent_prompt_id: None | str | Unset = UNSET
    recording_notice_interval_secs: int | None | Unset = UNSET
    cdr_archived_before: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        tenant_id = self.tenant_id

        recording_consent = self.recording_consent

        recording_consent_on_failure = self.recording_consent_on_failure

        subject_request_days = self.subject_request_days

        cdr_erasure = self.cdr_erasure

        cdr_archive_required = self.cdr_archive_required

        configured = self.configured

        updated_at = self.updated_at.isoformat()

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

        recording_consent_prompt_id: None | str | Unset
        if isinstance(self.recording_consent_prompt_id, Unset):
            recording_consent_prompt_id = UNSET
        else:
            recording_consent_prompt_id = self.recording_consent_prompt_id

        recording_notice_interval_secs: int | None | Unset
        if isinstance(self.recording_notice_interval_secs, Unset):
            recording_notice_interval_secs = UNSET
        else:
            recording_notice_interval_secs = self.recording_notice_interval_secs

        cdr_archived_before: int | None | Unset
        if isinstance(self.cdr_archived_before, Unset):
            cdr_archived_before = UNSET
        else:
            cdr_archived_before = self.cdr_archived_before


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "tenant_id": tenant_id,
            "recording_consent": recording_consent,
            "recording_consent_on_failure": recording_consent_on_failure,
            "subject_request_days": subject_request_days,
            "cdr_erasure": cdr_erasure,
            "cdr_archive_required": cdr_archive_required,
            "configured": configured,
            "updated_at": updated_at,
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
        if recording_consent_prompt_id is not UNSET:
            field_dict["recording_consent_prompt_id"] = recording_consent_prompt_id
        if recording_notice_interval_secs is not UNSET:
            field_dict["recording_notice_interval_secs"] = recording_notice_interval_secs
        if cdr_archived_before is not UNSET:
            field_dict["cdr_archived_before"] = cdr_archived_before

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        tenant_id = d.pop("tenant_id")

        recording_consent = d.pop("recording_consent")

        recording_consent_on_failure = d.pop("recording_consent_on_failure")

        subject_request_days = d.pop("subject_request_days")

        cdr_erasure = d.pop("cdr_erasure")

        cdr_archive_required = d.pop("cdr_archive_required")

        configured = d.pop("configured")

        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




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


        def _parse_recording_consent_prompt_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        recording_consent_prompt_id = _parse_recording_consent_prompt_id(d.pop("recording_consent_prompt_id", UNSET))


        def _parse_recording_notice_interval_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        recording_notice_interval_secs = _parse_recording_notice_interval_secs(d.pop("recording_notice_interval_secs", UNSET))


        def _parse_cdr_archived_before(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        cdr_archived_before = _parse_cdr_archived_before(d.pop("cdr_archived_before", UNSET))


        policy_response = cls(
            tenant_id=tenant_id,
            recording_consent=recording_consent,
            recording_consent_on_failure=recording_consent_on_failure,
            subject_request_days=subject_request_days,
            cdr_erasure=cdr_erasure,
            cdr_archive_required=cdr_archive_required,
            configured=configured,
            updated_at=updated_at,
            cdr_retention_days=cdr_retention_days,
            sip_trace_retention_days=sip_trace_retention_days,
            flow_trace_retention_days=flow_trace_retention_days,
            recording_retention_days=recording_retention_days,
            audit_retention_days=audit_retention_days,
            voicemail_retention_days=voicemail_retention_days,
            recording_consent_prompt_id=recording_consent_prompt_id,
            recording_notice_interval_secs=recording_notice_interval_secs,
            cdr_archived_before=cdr_archived_before,
        )


        policy_response.additional_properties = d
        return policy_response

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
