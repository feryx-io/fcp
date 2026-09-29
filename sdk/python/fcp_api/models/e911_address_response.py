from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="E911AddressResponse")



@_attrs_define
class E911AddressResponse:
    """ 
        Attributes:
            address1 (str):
            city (str):
            state (str):
            postal_code (str):
            country (str):
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
            address2 (None | str | Unset):
            validated (bool | Unset): Whether a validator has confirmed this address. Read-only: an
                address is validated by `POST …/e911/validate`, never by claiming
                it here (`docs/E911_BLOCK_2.md` §7).
            validated_at (datetime.datetime | None | Unset):
            validated_by (None | str | Unset): The validator that confirmed it, and the reference it gave.
            provider_reference (None | str | Unset):
     """

    address1: str
    city: str
    state: str
    postal_code: str
    country: str
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
    address2: None | str | Unset = UNSET
    validated: bool | Unset = UNSET
    validated_at: datetime.datetime | None | Unset = UNSET
    validated_by: None | str | Unset = UNSET
    provider_reference: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        address1 = self.address1

        city = self.city

        state = self.state

        postal_code = self.postal_code

        country = self.country

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

        address2: None | str | Unset
        if isinstance(self.address2, Unset):
            address2 = UNSET
        else:
            address2 = self.address2

        validated = self.validated

        validated_at: None | str | Unset
        if isinstance(self.validated_at, Unset):
            validated_at = UNSET
        elif isinstance(self.validated_at, datetime.datetime):
            validated_at = self.validated_at.isoformat()
        else:
            validated_at = self.validated_at

        validated_by: None | str | Unset
        if isinstance(self.validated_by, Unset):
            validated_by = UNSET
        else:
            validated_by = self.validated_by

        provider_reference: None | str | Unset
        if isinstance(self.provider_reference, Unset):
            provider_reference = UNSET
        else:
            provider_reference = self.provider_reference


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "address1": address1,
            "city": city,
            "state": state,
            "postal_code": postal_code,
            "country": country,
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
        if address2 is not UNSET:
            field_dict["address2"] = address2
        if validated is not UNSET:
            field_dict["validated"] = validated
        if validated_at is not UNSET:
            field_dict["validated_at"] = validated_at
        if validated_by is not UNSET:
            field_dict["validated_by"] = validated_by
        if provider_reference is not UNSET:
            field_dict["provider_reference"] = provider_reference

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        address1 = d.pop("address1")

        city = d.pop("city")

        state = d.pop("state")

        postal_code = d.pop("postal_code")

        country = d.pop("country")

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


        def _parse_address2(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        address2 = _parse_address2(d.pop("address2", UNSET))


        validated = d.pop("validated", UNSET)

        def _parse_validated_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                validated_at_type_0 = datetime.datetime.fromisoformat(data)



                return validated_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        validated_at = _parse_validated_at(d.pop("validated_at", UNSET))


        def _parse_validated_by(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        validated_by = _parse_validated_by(d.pop("validated_by", UNSET))


        def _parse_provider_reference(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        provider_reference = _parse_provider_reference(d.pop("provider_reference", UNSET))


        e911_address_response = cls(
            address1=address1,
            city=city,
            state=state,
            postal_code=postal_code,
            country=country,
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
            address2=address2,
            validated=validated,
            validated_at=validated_at,
            validated_by=validated_by,
            provider_reference=provider_reference,
        )


        e911_address_response.additional_properties = d
        return e911_address_response

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
