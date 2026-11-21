from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="StreamStateResponse")



@_attrs_define
class StreamStateResponse:
    """ A webinar's stream, as the API shows it.

        Attributes:
            state (str): `off` (the meeting does not stream), `waiting` (it will, once live),
                `live`, or `ended` (its newest stream ended, kept for replay).
            recording_id (None | str | Unset): The newest stream's recording.
            sitting_id (None | str | Unset): The sitting it streamed.
            started_at (datetime.datetime | None | Unset):
            ended_at (datetime.datetime | None | Unset):
            duration_secs (int | None | Unset):
     """

    state: str
    recording_id: None | str | Unset = UNSET
    sitting_id: None | str | Unset = UNSET
    started_at: datetime.datetime | None | Unset = UNSET
    ended_at: datetime.datetime | None | Unset = UNSET
    duration_secs: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        state = self.state

        recording_id: None | str | Unset
        if isinstance(self.recording_id, Unset):
            recording_id = UNSET
        else:
            recording_id = self.recording_id

        sitting_id: None | str | Unset
        if isinstance(self.sitting_id, Unset):
            sitting_id = UNSET
        else:
            sitting_id = self.sitting_id

        started_at: None | str | Unset
        if isinstance(self.started_at, Unset):
            started_at = UNSET
        elif isinstance(self.started_at, datetime.datetime):
            started_at = self.started_at.isoformat()
        else:
            started_at = self.started_at

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


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "state": state,
        })
        if recording_id is not UNSET:
            field_dict["recording_id"] = recording_id
        if sitting_id is not UNSET:
            field_dict["sitting_id"] = sitting_id
        if started_at is not UNSET:
            field_dict["started_at"] = started_at
        if ended_at is not UNSET:
            field_dict["ended_at"] = ended_at
        if duration_secs is not UNSET:
            field_dict["duration_secs"] = duration_secs

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        state = d.pop("state")

        def _parse_recording_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        recording_id = _parse_recording_id(d.pop("recording_id", UNSET))


        def _parse_sitting_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        sitting_id = _parse_sitting_id(d.pop("sitting_id", UNSET))


        def _parse_started_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                started_at_type_0 = datetime.datetime.fromisoformat(data)



                return started_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        started_at = _parse_started_at(d.pop("started_at", UNSET))


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


        stream_state_response = cls(
            state=state,
            recording_id=recording_id,
            sitting_id=sitting_id,
            started_at=started_at,
            ended_at=ended_at,
            duration_secs=duration_secs,
        )


        stream_state_response.additional_properties = d
        return stream_state_response

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
