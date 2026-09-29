from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="MeMessageResponse")



@_attrs_define
class MeMessageResponse:
    """ Message response.

        Attributes:
            id (str):
            mailbox_id (str):
            caller (str):
            status (str):
            priority (str):
            duration (int):
            recording_path (str):
            created_at (datetime.datetime):
            caller_name (None | str | Unset):
            transcription (None | str | Unset):
            accessed_at (datetime.datetime | None | Unset):
     """

    id: str
    mailbox_id: str
    caller: str
    status: str
    priority: str
    duration: int
    recording_path: str
    created_at: datetime.datetime
    caller_name: None | str | Unset = UNSET
    transcription: None | str | Unset = UNSET
    accessed_at: datetime.datetime | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        mailbox_id = self.mailbox_id

        caller = self.caller

        status = self.status

        priority = self.priority

        duration = self.duration

        recording_path = self.recording_path

        created_at = self.created_at.isoformat()

        caller_name: None | str | Unset
        if isinstance(self.caller_name, Unset):
            caller_name = UNSET
        else:
            caller_name = self.caller_name

        transcription: None | str | Unset
        if isinstance(self.transcription, Unset):
            transcription = UNSET
        else:
            transcription = self.transcription

        accessed_at: None | str | Unset
        if isinstance(self.accessed_at, Unset):
            accessed_at = UNSET
        elif isinstance(self.accessed_at, datetime.datetime):
            accessed_at = self.accessed_at.isoformat()
        else:
            accessed_at = self.accessed_at


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "mailbox_id": mailbox_id,
            "caller": caller,
            "status": status,
            "priority": priority,
            "duration": duration,
            "recording_path": recording_path,
            "created_at": created_at,
        })
        if caller_name is not UNSET:
            field_dict["caller_name"] = caller_name
        if transcription is not UNSET:
            field_dict["transcription"] = transcription
        if accessed_at is not UNSET:
            field_dict["accessed_at"] = accessed_at

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        mailbox_id = d.pop("mailbox_id")

        caller = d.pop("caller")

        status = d.pop("status")

        priority = d.pop("priority")

        duration = d.pop("duration")

        recording_path = d.pop("recording_path")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        def _parse_caller_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        caller_name = _parse_caller_name(d.pop("caller_name", UNSET))


        def _parse_transcription(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        transcription = _parse_transcription(d.pop("transcription", UNSET))


        def _parse_accessed_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                accessed_at_type_0 = datetime.datetime.fromisoformat(data)



                return accessed_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        accessed_at = _parse_accessed_at(d.pop("accessed_at", UNSET))


        me_message_response = cls(
            id=id,
            mailbox_id=mailbox_id,
            caller=caller,
            status=status,
            priority=priority,
            duration=duration,
            recording_path=recording_path,
            created_at=created_at,
            caller_name=caller_name,
            transcription=transcription,
            accessed_at=accessed_at,
        )


        me_message_response.additional_properties = d
        return me_message_response

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
