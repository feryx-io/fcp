from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="AttendanceLeg")



@_attrs_define
class AttendanceLeg:
    """ One leg of a sitting, from its record.

        Attributes:
            from_ (str): Where they called from.
            joined_at (datetime.datetime):
            duration_secs (int):
            lobby (bool): They left from the lobby, never let in.
            questions (int): Questions they asked, and chat messages they posted, in the sitting.
            messages (int):
            participant_id (None | str | Unset):
            name (None | str | Unset):
            role (None | str | Unset): `host`, `panelist`, `attendee` or `guest`.
            registrant (None | str | Unset): The registration they came by.
            left_at (datetime.datetime | None | Unset):
     """

    from_: str
    joined_at: datetime.datetime
    duration_secs: int
    lobby: bool
    questions: int
    messages: int
    participant_id: None | str | Unset = UNSET
    name: None | str | Unset = UNSET
    role: None | str | Unset = UNSET
    registrant: None | str | Unset = UNSET
    left_at: datetime.datetime | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from_ = self.from_

        joined_at = self.joined_at.isoformat()

        duration_secs = self.duration_secs

        lobby = self.lobby

        questions = self.questions

        messages = self.messages

        participant_id: None | str | Unset
        if isinstance(self.participant_id, Unset):
            participant_id = UNSET
        else:
            participant_id = self.participant_id

        name: None | str | Unset
        if isinstance(self.name, Unset):
            name = UNSET
        else:
            name = self.name

        role: None | str | Unset
        if isinstance(self.role, Unset):
            role = UNSET
        else:
            role = self.role

        registrant: None | str | Unset
        if isinstance(self.registrant, Unset):
            registrant = UNSET
        else:
            registrant = self.registrant

        left_at: None | str | Unset
        if isinstance(self.left_at, Unset):
            left_at = UNSET
        elif isinstance(self.left_at, datetime.datetime):
            left_at = self.left_at.isoformat()
        else:
            left_at = self.left_at


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "from": from_,
            "joined_at": joined_at,
            "duration_secs": duration_secs,
            "lobby": lobby,
            "questions": questions,
            "messages": messages,
        })
        if participant_id is not UNSET:
            field_dict["participant_id"] = participant_id
        if name is not UNSET:
            field_dict["name"] = name
        if role is not UNSET:
            field_dict["role"] = role
        if registrant is not UNSET:
            field_dict["registrant"] = registrant
        if left_at is not UNSET:
            field_dict["left_at"] = left_at

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        from_ = d.pop("from")

        joined_at = datetime.datetime.fromisoformat(d.pop("joined_at"))




        duration_secs = d.pop("duration_secs")

        lobby = d.pop("lobby")

        questions = d.pop("questions")

        messages = d.pop("messages")

        def _parse_participant_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        participant_id = _parse_participant_id(d.pop("participant_id", UNSET))


        def _parse_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        name = _parse_name(d.pop("name", UNSET))


        def _parse_role(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        role = _parse_role(d.pop("role", UNSET))


        def _parse_registrant(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        registrant = _parse_registrant(d.pop("registrant", UNSET))


        def _parse_left_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                left_at_type_0 = datetime.datetime.fromisoformat(data)



                return left_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        left_at = _parse_left_at(d.pop("left_at", UNSET))


        attendance_leg = cls(
            from_=from_,
            joined_at=joined_at,
            duration_secs=duration_secs,
            lobby=lobby,
            questions=questions,
            messages=messages,
            participant_id=participant_id,
            name=name,
            role=role,
            registrant=registrant,
            left_at=left_at,
        )


        attendance_leg.additional_properties = d
        return attendance_leg

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
