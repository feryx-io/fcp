from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="UpdateMyPresenceRequest")



@_attrs_define
class UpdateMyPresenceRequest:
    """ What may change. `status` sets the chosen status (a built-in state's
    name or one of the user's statuses' ids) with `note` and `until`, both
    cleared when left out; without `status`, a `note` or `until` given
    changes only that (`null` clears it). `idle` is an app's word that its
    person is idle (`true`) or back (`false`), lasting 15 minutes unless
    repeated.

        Attributes:
            status (None | str | Unset):
            note (None | str | Unset):
            until (datetime.datetime | None | Unset): RFC 3339, in the future and within 7 days.
            idle (bool | None | Unset):
     """

    status: None | str | Unset = UNSET
    note: None | str | Unset = UNSET
    until: datetime.datetime | None | Unset = UNSET
    idle: bool | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        status: None | str | Unset
        if isinstance(self.status, Unset):
            status = UNSET
        else:
            status = self.status

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

        idle: bool | None | Unset
        if isinstance(self.idle, Unset):
            idle = UNSET
        else:
            idle = self.idle


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if status is not UNSET:
            field_dict["status"] = status
        if note is not UNSET:
            field_dict["note"] = note
        if until is not UNSET:
            field_dict["until"] = until
        if idle is not UNSET:
            field_dict["idle"] = idle

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_status(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        status = _parse_status(d.pop("status", UNSET))


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


        def _parse_idle(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        idle = _parse_idle(d.pop("idle", UNSET))


        update_my_presence_request = cls(
            status=status,
            note=note,
            until=until,
            idle=idle,
        )


        update_my_presence_request.additional_properties = d
        return update_my_presence_request

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
