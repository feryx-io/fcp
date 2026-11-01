from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.status_state import StatusState
from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.presence import Presence





T = TypeVar("T", bound="UserPresenceResponse")



@_attrs_define
class UserPresenceResponse:
    """ A user's presence, as an administrator reads it.

        Attributes:
            username (str):
            presence (Presence): What others see of a person.
            status (str):
            status_state (StatusState): The state a status carries: what colour it is and what others' phones
                are told.
            status_label (None | str | Unset):
            note (None | str | Unset):
            until (datetime.datetime | None | Unset):
     """

    username: str
    presence: Presence
    status: str
    status_state: StatusState
    status_label: None | str | Unset = UNSET
    note: None | str | Unset = UNSET
    until: datetime.datetime | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.presence import Presence # noqa: PLC0415
        username = self.username

        presence = self.presence.to_dict()

        status = self.status

        status_state = self.status_state.value

        status_label: None | str | Unset
        if isinstance(self.status_label, Unset):
            status_label = UNSET
        else:
            status_label = self.status_label

        note: None | str | Unset
        if isinstance(self.note, Unset):
            note = UNSET
        else:
            note = self.note

        until: None | str | Unset
        if isinstance(self.until, Unset):
            until = UNSET
        elif isinstance(self.until, datetime.datetime):
            until = self.until.isoformat()
        else:
            until = self.until


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "username": username,
            "presence": presence,
            "status": status,
            "status_state": status_state,
        })
        if status_label is not UNSET:
            field_dict["status_label"] = status_label
        if note is not UNSET:
            field_dict["note"] = note
        if until is not UNSET:
            field_dict["until"] = until

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.presence import Presence # noqa: PLC0415
        d = dict(src_dict)
        username = d.pop("username")

        presence = Presence.from_dict(d.pop("presence"))




        status = d.pop("status")

        status_state = StatusState(d.pop("status_state"))




        def _parse_status_label(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        status_label = _parse_status_label(d.pop("status_label", UNSET))


        def _parse_note(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        note = _parse_note(d.pop("note", UNSET))


        def _parse_until(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                until_type_0 = datetime.datetime.fromisoformat(data)



                return until_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        until = _parse_until(d.pop("until", UNSET))


        user_presence_response = cls(
            username=username,
            presence=presence,
            status=status,
            status_state=status_state,
            status_label=status_label,
            note=note,
            until=until,
        )


        user_presence_response.additional_properties = d
        return user_presence_response

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
