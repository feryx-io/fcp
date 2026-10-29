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
  from ..models.own_status_response import OwnStatusResponse
  from ..models.presence import Presence





T = TypeVar("T", bound="MyPresenceResponse")



@_attrs_define
class MyPresenceResponse:
    """ A user's presence: what others see, what they chose, and their own
    statuses to choose from.

        Attributes:
            presence (Presence): What others see of a person.
            status (str): What they chose: a built-in state's name or one of `statuses`' ids.
            status_state (StatusState): The state a status carries: what colour it is and what others' phones
                are told.
            statuses (list[OwnStatusResponse]):
            note (None | str | Unset):
            until (datetime.datetime | None | Unset): When the chosen status clears back to available.
     """

    presence: Presence
    status: str
    status_state: StatusState
    statuses: list[OwnStatusResponse]
    note: None | str | Unset = UNSET
    until: datetime.datetime | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.own_status_response import OwnStatusResponse # noqa: PLC0415
        from ..models.presence import Presence # noqa: PLC0415
        presence = self.presence.to_dict()

        status = self.status

        status_state = self.status_state.value

        statuses = []
        for statuses_item_data in self.statuses:
            statuses_item = statuses_item_data.to_dict()
            statuses.append(statuses_item)



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
            "presence": presence,
            "status": status,
            "status_state": status_state,
            "statuses": statuses,
        })
        if note is not UNSET:
            field_dict["note"] = note
        if until is not UNSET:
            field_dict["until"] = until

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.own_status_response import OwnStatusResponse # noqa: PLC0415
        from ..models.presence import Presence # noqa: PLC0415
        d = dict(src_dict)
        presence = Presence.from_dict(d.pop("presence"))




        status = d.pop("status")

        status_state = StatusState(d.pop("status_state"))




        statuses = []
        _statuses = d.pop("statuses")
        for statuses_item_data in (_statuses):
            statuses_item = OwnStatusResponse.from_dict(statuses_item_data)



            statuses.append(statuses_item)


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


        my_presence_response = cls(
            presence=presence,
            status=status,
            status_state=status_state,
            statuses=statuses,
            note=note,
            until=until,
        )


        my_presence_response.additional_properties = d
        return my_presence_response

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
