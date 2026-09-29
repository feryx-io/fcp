from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast
import datetime






T = TypeVar("T", bound="RetentionRunResponse")



@_attrs_define
class RetentionRunResponse:
    """ 
        Attributes:
            tenant_id (str):
            ran_at (datetime.datetime):
            cdrs_deleted (int):
            sip_traces_deleted (int):
            flow_traces_deleted (int):
            recordings_deleted (int):
            conference_recordings_deleted (int): Conference recordings whose registry row the run removed.
            voicemail_messages_deleted (int):
            audit_events_deleted (int):
            total_deleted (int):
            errors (list[str]):
     """

    tenant_id: str
    ran_at: datetime.datetime
    cdrs_deleted: int
    sip_traces_deleted: int
    flow_traces_deleted: int
    recordings_deleted: int
    conference_recordings_deleted: int
    voicemail_messages_deleted: int
    audit_events_deleted: int
    total_deleted: int
    errors: list[str]
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        tenant_id = self.tenant_id

        ran_at = self.ran_at.isoformat()

        cdrs_deleted = self.cdrs_deleted

        sip_traces_deleted = self.sip_traces_deleted

        flow_traces_deleted = self.flow_traces_deleted

        recordings_deleted = self.recordings_deleted

        conference_recordings_deleted = self.conference_recordings_deleted

        voicemail_messages_deleted = self.voicemail_messages_deleted

        audit_events_deleted = self.audit_events_deleted

        total_deleted = self.total_deleted

        errors = self.errors




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "tenant_id": tenant_id,
            "ran_at": ran_at,
            "cdrs_deleted": cdrs_deleted,
            "sip_traces_deleted": sip_traces_deleted,
            "flow_traces_deleted": flow_traces_deleted,
            "recordings_deleted": recordings_deleted,
            "conference_recordings_deleted": conference_recordings_deleted,
            "voicemail_messages_deleted": voicemail_messages_deleted,
            "audit_events_deleted": audit_events_deleted,
            "total_deleted": total_deleted,
            "errors": errors,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        tenant_id = d.pop("tenant_id")

        ran_at = datetime.datetime.fromisoformat(d.pop("ran_at"))




        cdrs_deleted = d.pop("cdrs_deleted")

        sip_traces_deleted = d.pop("sip_traces_deleted")

        flow_traces_deleted = d.pop("flow_traces_deleted")

        recordings_deleted = d.pop("recordings_deleted")

        conference_recordings_deleted = d.pop("conference_recordings_deleted")

        voicemail_messages_deleted = d.pop("voicemail_messages_deleted")

        audit_events_deleted = d.pop("audit_events_deleted")

        total_deleted = d.pop("total_deleted")

        errors = cast(list[str], d.pop("errors"))


        retention_run_response = cls(
            tenant_id=tenant_id,
            ran_at=ran_at,
            cdrs_deleted=cdrs_deleted,
            sip_traces_deleted=sip_traces_deleted,
            flow_traces_deleted=flow_traces_deleted,
            recordings_deleted=recordings_deleted,
            conference_recordings_deleted=conference_recordings_deleted,
            voicemail_messages_deleted=voicemail_messages_deleted,
            audit_events_deleted=audit_events_deleted,
            total_deleted=total_deleted,
            errors=errors,
        )


        retention_run_response.additional_properties = d
        return retention_run_response

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
