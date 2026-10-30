from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="ParkingLotResponse")



@_attrs_define
class ParkingLotResponse:
    """ 
        Attributes:
            id (str):
            tenant_id (str):
            name (str):
            first (str): The first and last slot: digit strings of one width.
            last (str):
            slots (int):
            timeout_secs (int):
            enabled (bool):
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            park_number (None | str | Unset):
            fallback (None | str | Unset): Where a call goes when its parker does not take it back
                (`ext:1001`, `queue:support`, `voicemail`, …).
            music (None | str | Unset):
            location_id (None | str | Unset):
     """

    id: str
    tenant_id: str
    name: str
    first: str
    last: str
    slots: int
    timeout_secs: int
    enabled: bool
    created_at: datetime.datetime
    updated_at: datetime.datetime
    park_number: None | str | Unset = UNSET
    fallback: None | str | Unset = UNSET
    music: None | str | Unset = UNSET
    location_id: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        tenant_id = self.tenant_id

        name = self.name

        first = self.first

        last = self.last

        slots = self.slots

        timeout_secs = self.timeout_secs

        enabled = self.enabled

        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

        park_number: None | str | Unset
        if isinstance(self.park_number, Unset):
            park_number = UNSET
        else:
            park_number = self.park_number

        fallback: None | str | Unset
        if isinstance(self.fallback, Unset):
            fallback = UNSET
        else:
            fallback = self.fallback

        music: None | str | Unset
        if isinstance(self.music, Unset):
            music = UNSET
        else:
            music = self.music

        location_id: None | str | Unset
        if isinstance(self.location_id, Unset):
            location_id = UNSET
        else:
            location_id = self.location_id


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "name": name,
            "first": first,
            "last": last,
            "slots": slots,
            "timeout_secs": timeout_secs,
            "enabled": enabled,
            "created_at": created_at,
            "updated_at": updated_at,
        })
        if park_number is not UNSET:
            field_dict["park_number"] = park_number
        if fallback is not UNSET:
            field_dict["fallback"] = fallback
        if music is not UNSET:
            field_dict["music"] = music
        if location_id is not UNSET:
            field_dict["location_id"] = location_id

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        name = d.pop("name")

        first = d.pop("first")

        last = d.pop("last")

        slots = d.pop("slots")

        timeout_secs = d.pop("timeout_secs")

        enabled = d.pop("enabled")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_park_number(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        park_number = _parse_park_number(d.pop("park_number", UNSET))


        def _parse_fallback(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        fallback = _parse_fallback(d.pop("fallback", UNSET))


        def _parse_music(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        music = _parse_music(d.pop("music", UNSET))


        def _parse_location_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        location_id = _parse_location_id(d.pop("location_id", UNSET))


        parking_lot_response = cls(
            id=id,
            tenant_id=tenant_id,
            name=name,
            first=first,
            last=last,
            slots=slots,
            timeout_secs=timeout_secs,
            enabled=enabled,
            created_at=created_at,
            updated_at=updated_at,
            park_number=park_number,
            fallback=fallback,
            music=music,
            location_id=location_id,
        )


        parking_lot_response.additional_properties = d
        return parking_lot_response

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
