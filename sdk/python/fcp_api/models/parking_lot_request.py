from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="ParkingLotRequest")



@_attrs_define
class ParkingLotRequest:
    """ What a lot is written as; a `PUT` replaces it whole.

        Attributes:
            name (str):
            first (str):
            last (str):
            park_number (None | str | Unset):
            timeout_secs (int | None | Unset): Seconds; 60 when unset.
            fallback (None | str | Unset):
            music (None | str | Unset):
            location_id (None | str | Unset):
            enabled (bool | Unset):
     """

    name: str
    first: str
    last: str
    park_number: None | str | Unset = UNSET
    timeout_secs: int | None | Unset = UNSET
    fallback: None | str | Unset = UNSET
    music: None | str | Unset = UNSET
    location_id: None | str | Unset = UNSET
    enabled: bool | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        name = self.name

        first = self.first

        last = self.last

        park_number: None | str | Unset
        if isinstance(self.park_number, Unset):
            park_number = UNSET
        else:
            park_number = self.park_number

        timeout_secs: int | None | Unset
        if isinstance(self.timeout_secs, Unset):
            timeout_secs = UNSET
        else:
            timeout_secs = self.timeout_secs

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

        enabled = self.enabled


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "name": name,
            "first": first,
            "last": last,
        })
        if park_number is not UNSET:
            field_dict["park_number"] = park_number
        if timeout_secs is not UNSET:
            field_dict["timeout_secs"] = timeout_secs
        if fallback is not UNSET:
            field_dict["fallback"] = fallback
        if music is not UNSET:
            field_dict["music"] = music
        if location_id is not UNSET:
            field_dict["location_id"] = location_id
        if enabled is not UNSET:
            field_dict["enabled"] = enabled

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        name = d.pop("name")

        first = d.pop("first")

        last = d.pop("last")

        def _parse_park_number(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        park_number = _parse_park_number(d.pop("park_number", UNSET))


        def _parse_timeout_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        timeout_secs = _parse_timeout_secs(d.pop("timeout_secs", UNSET))


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


        enabled = d.pop("enabled", UNSET)

        parking_lot_request = cls(
            name=name,
            first=first,
            last=last,
            park_number=park_number,
            timeout_secs=timeout_secs,
            fallback=fallback,
            music=music,
            location_id=location_id,
            enabled=enabled,
        )

        return parking_lot_request

