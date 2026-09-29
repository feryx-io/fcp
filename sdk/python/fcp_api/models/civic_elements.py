from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="CivicElements")



@_attrs_define
class CivicElements:
    """ The RFC 5139 civic address elements beyond city, state, postal code and
    country, which [`E911Address`] carries as fields of its own.

        Attributes:
            house_number (None | str | Unset): `HNO`: the numeric part of the house number (`1455`).
            house_number_suffix (None | str | Unset): `HNS`: a house number suffix (`A`, `1/2`).
            street_predirection (None | str | Unset): `PRD`: a leading street direction (`N`).
            street_name (None | str | Unset): `RD`: the street name (`Market`).
            street_suffix (None | str | Unset): `STS`: the street suffix (`St`).
            street_postdirection (None | str | Unset): `POD`: a trailing street direction (`SW`).
            additional (None | str | Unset): `LOC`: additional location information (`Room 4B`).
            floor (None | str | Unset): `FLR`: the floor.
            unit (None | str | Unset): `UNIT`: a unit or suite.
            room (None | str | Unset): `ROOM`.
            building (None | str | Unset): `BLD`: the building.
            name (None | str | Unset): `NAM`: the name of the place (`Example HQ`).
     """

    house_number: None | str | Unset = UNSET
    house_number_suffix: None | str | Unset = UNSET
    street_predirection: None | str | Unset = UNSET
    street_name: None | str | Unset = UNSET
    street_suffix: None | str | Unset = UNSET
    street_postdirection: None | str | Unset = UNSET
    additional: None | str | Unset = UNSET
    floor: None | str | Unset = UNSET
    unit: None | str | Unset = UNSET
    room: None | str | Unset = UNSET
    building: None | str | Unset = UNSET
    name: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        house_number: None | str | Unset
        if isinstance(self.house_number, Unset):
            house_number = UNSET
        else:
            house_number = self.house_number

        house_number_suffix: None | str | Unset
        if isinstance(self.house_number_suffix, Unset):
            house_number_suffix = UNSET
        else:
            house_number_suffix = self.house_number_suffix

        street_predirection: None | str | Unset
        if isinstance(self.street_predirection, Unset):
            street_predirection = UNSET
        else:
            street_predirection = self.street_predirection

        street_name: None | str | Unset
        if isinstance(self.street_name, Unset):
            street_name = UNSET
        else:
            street_name = self.street_name

        street_suffix: None | str | Unset
        if isinstance(self.street_suffix, Unset):
            street_suffix = UNSET
        else:
            street_suffix = self.street_suffix

        street_postdirection: None | str | Unset
        if isinstance(self.street_postdirection, Unset):
            street_postdirection = UNSET
        else:
            street_postdirection = self.street_postdirection

        additional: None | str | Unset
        if isinstance(self.additional, Unset):
            additional = UNSET
        else:
            additional = self.additional

        floor: None | str | Unset
        if isinstance(self.floor, Unset):
            floor = UNSET
        else:
            floor = self.floor

        unit: None | str | Unset
        if isinstance(self.unit, Unset):
            unit = UNSET
        else:
            unit = self.unit

        room: None | str | Unset
        if isinstance(self.room, Unset):
            room = UNSET
        else:
            room = self.room

        building: None | str | Unset
        if isinstance(self.building, Unset):
            building = UNSET
        else:
            building = self.building

        name: None | str | Unset
        if isinstance(self.name, Unset):
            name = UNSET
        else:
            name = self.name


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if house_number is not UNSET:
            field_dict["house_number"] = house_number
        if house_number_suffix is not UNSET:
            field_dict["house_number_suffix"] = house_number_suffix
        if street_predirection is not UNSET:
            field_dict["street_predirection"] = street_predirection
        if street_name is not UNSET:
            field_dict["street_name"] = street_name
        if street_suffix is not UNSET:
            field_dict["street_suffix"] = street_suffix
        if street_postdirection is not UNSET:
            field_dict["street_postdirection"] = street_postdirection
        if additional is not UNSET:
            field_dict["additional"] = additional
        if floor is not UNSET:
            field_dict["floor"] = floor
        if unit is not UNSET:
            field_dict["unit"] = unit
        if room is not UNSET:
            field_dict["room"] = room
        if building is not UNSET:
            field_dict["building"] = building
        if name is not UNSET:
            field_dict["name"] = name

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_house_number(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        house_number = _parse_house_number(d.pop("house_number", UNSET))


        def _parse_house_number_suffix(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        house_number_suffix = _parse_house_number_suffix(d.pop("house_number_suffix", UNSET))


        def _parse_street_predirection(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        street_predirection = _parse_street_predirection(d.pop("street_predirection", UNSET))


        def _parse_street_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        street_name = _parse_street_name(d.pop("street_name", UNSET))


        def _parse_street_suffix(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        street_suffix = _parse_street_suffix(d.pop("street_suffix", UNSET))


        def _parse_street_postdirection(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        street_postdirection = _parse_street_postdirection(d.pop("street_postdirection", UNSET))


        def _parse_additional(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        additional = _parse_additional(d.pop("additional", UNSET))


        def _parse_floor(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        floor = _parse_floor(d.pop("floor", UNSET))


        def _parse_unit(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        unit = _parse_unit(d.pop("unit", UNSET))


        def _parse_room(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        room = _parse_room(d.pop("room", UNSET))


        def _parse_building(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        building = _parse_building(d.pop("building", UNSET))


        def _parse_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        name = _parse_name(d.pop("name", UNSET))


        civic_elements = cls(
            house_number=house_number,
            house_number_suffix=house_number_suffix,
            street_predirection=street_predirection,
            street_name=street_name,
            street_suffix=street_suffix,
            street_postdirection=street_postdirection,
            additional=additional,
            floor=floor,
            unit=unit,
            room=room,
            building=building,
            name=name,
        )


        civic_elements.additional_properties = d
        return civic_elements

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
