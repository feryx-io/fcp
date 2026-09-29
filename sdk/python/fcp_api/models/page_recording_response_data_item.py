from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="PageRecordingResponseDataItem")



@_attrs_define
class PageRecordingResponseDataItem:
    """ 
        Attributes:
            id (str):
            tenant_id (str):
            room_number (str):
            node_id (str):
            file_name (str): The file's name; the path on the node is not a caller's business.
            format_ (str):
            trigger (str):
            consent (str):
            started_at (datetime.datetime):
            participants (int):
            running (bool): Still being written.
            download_url (str): Where the bytes are, on this API.
            meeting_id (None | str | Unset):
            ended_at (datetime.datetime | None | Unset):
            duration_secs (int | None | Unset):
            size_bytes (int | None | Unset):
     """

    id: str
    tenant_id: str
    room_number: str
    node_id: str
    file_name: str
    format_: str
    trigger: str
    consent: str
    started_at: datetime.datetime
    participants: int
    running: bool
    download_url: str
    meeting_id: None | str | Unset = UNSET
    ended_at: datetime.datetime | None | Unset = UNSET
    duration_secs: int | None | Unset = UNSET
    size_bytes: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        tenant_id = self.tenant_id

        room_number = self.room_number

        node_id = self.node_id

        file_name = self.file_name

        format_ = self.format_

        trigger = self.trigger

        consent = self.consent

        started_at = self.started_at.isoformat()

        participants = self.participants

        running = self.running

        download_url = self.download_url

        meeting_id: None | str | Unset
        if isinstance(self.meeting_id, Unset):
            meeting_id = UNSET
        else:
            meeting_id = self.meeting_id

        ended_at: None | str | Unset
        if isinstance(self.ended_at, Unset):
            ended_at = UNSET
        elif isinstance(self.ended_at, datetime.datetime):
            ended_at = self.ended_at.isoformat()
        else:
            ended_at = self.ended_at

        duration_secs: int | None | Unset
        if isinstance(self.duration_secs, Unset):
            duration_secs = UNSET
        else:
            duration_secs = self.duration_secs

        size_bytes: int | None | Unset
        if isinstance(self.size_bytes, Unset):
            size_bytes = UNSET
        else:
            size_bytes = self.size_bytes


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "room_number": room_number,
            "node_id": node_id,
            "file_name": file_name,
            "format": format_,
            "trigger": trigger,
            "consent": consent,
            "started_at": started_at,
            "participants": participants,
            "running": running,
            "download_url": download_url,
        })
        if meeting_id is not UNSET:
            field_dict["meeting_id"] = meeting_id
        if ended_at is not UNSET:
            field_dict["ended_at"] = ended_at
        if duration_secs is not UNSET:
            field_dict["duration_secs"] = duration_secs
        if size_bytes is not UNSET:
            field_dict["size_bytes"] = size_bytes

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        room_number = d.pop("room_number")

        node_id = d.pop("node_id")

        file_name = d.pop("file_name")

        format_ = d.pop("format")

        trigger = d.pop("trigger")

        consent = d.pop("consent")

        started_at = datetime.datetime.fromisoformat(d.pop("started_at"))




        participants = d.pop("participants")

        running = d.pop("running")

        download_url = d.pop("download_url")

        def _parse_meeting_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        meeting_id = _parse_meeting_id(d.pop("meeting_id", UNSET))


        def _parse_ended_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                ended_at_type_0 = datetime.datetime.fromisoformat(data)



                return ended_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        ended_at = _parse_ended_at(d.pop("ended_at", UNSET))


        def _parse_duration_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        duration_secs = _parse_duration_secs(d.pop("duration_secs", UNSET))


        def _parse_size_bytes(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        size_bytes = _parse_size_bytes(d.pop("size_bytes", UNSET))


        page_recording_response_data_item = cls(
            id=id,
            tenant_id=tenant_id,
            room_number=room_number,
            node_id=node_id,
            file_name=file_name,
            format_=format_,
            trigger=trigger,
            consent=consent,
            started_at=started_at,
            participants=participants,
            running=running,
            download_url=download_url,
            meeting_id=meeting_id,
            ended_at=ended_at,
            duration_secs=duration_secs,
            size_bytes=size_bytes,
        )


        page_recording_response_data_item.additional_properties = d
        return page_recording_response_data_item

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
