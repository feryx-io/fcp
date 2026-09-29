from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.civic_elements import CivicElements





T = TypeVar("T", bound="CreateLocationRequest")



@_attrs_define
class CreateLocationRequest:
    """ Create location request.

        Attributes:
            name (str):
            tenant_id (str | Unset): A TenantId: `tenant-<uuid>`, or the bare UUID
            description (None | str | Unset):
            address_line1 (None | str | Unset):
            address_line2 (None | str | Unset):
            city (None | str | Unset):
            state (None | str | Unset):
            postal_code (None | str | Unset):
            country (None | str | Unset):
            floor (None | str | Unset):
            building (None | str | Unset):
            latitude (float | None | Unset):
            longitude (float | None | Unset):
            phone_number (None | str | Unset):
            timezone (None | str | Unset):
            emergency_contact_name (None | str | Unset):
            emergency_contact_phone (None | str | Unset):
            e911_callback_number (None | str | Unset):
            civic (CivicElements | None | Unset):
     """

    name: str
    tenant_id: str | Unset = UNSET
    description: None | str | Unset = UNSET
    address_line1: None | str | Unset = UNSET
    address_line2: None | str | Unset = UNSET
    city: None | str | Unset = UNSET
    state: None | str | Unset = UNSET
    postal_code: None | str | Unset = UNSET
    country: None | str | Unset = UNSET
    floor: None | str | Unset = UNSET
    building: None | str | Unset = UNSET
    latitude: float | None | Unset = UNSET
    longitude: float | None | Unset = UNSET
    phone_number: None | str | Unset = UNSET
    timezone: None | str | Unset = UNSET
    emergency_contact_name: None | str | Unset = UNSET
    emergency_contact_phone: None | str | Unset = UNSET
    e911_callback_number: None | str | Unset = UNSET
    civic: CivicElements | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.civic_elements import CivicElements # noqa: PLC0415
        name = self.name

        tenant_id = self.tenant_id

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description

        address_line1: None | str | Unset
        if isinstance(self.address_line1, Unset):
            address_line1 = UNSET
        else:
            address_line1 = self.address_line1

        address_line2: None | str | Unset
        if isinstance(self.address_line2, Unset):
            address_line2 = UNSET
        else:
            address_line2 = self.address_line2

        city: None | str | Unset
        if isinstance(self.city, Unset):
            city = UNSET
        else:
            city = self.city

        state: None | str | Unset
        if isinstance(self.state, Unset):
            state = UNSET
        else:
            state = self.state

        postal_code: None | str | Unset
        if isinstance(self.postal_code, Unset):
            postal_code = UNSET
        else:
            postal_code = self.postal_code

        country: None | str | Unset
        if isinstance(self.country, Unset):
            country = UNSET
        else:
            country = self.country

        floor: None | str | Unset
        if isinstance(self.floor, Unset):
            floor = UNSET
        else:
            floor = self.floor

        building: None | str | Unset
        if isinstance(self.building, Unset):
            building = UNSET
        else:
            building = self.building

        latitude: float | None | Unset
        if isinstance(self.latitude, Unset):
            latitude = UNSET
        else:
            latitude = self.latitude

        longitude: float | None | Unset
        if isinstance(self.longitude, Unset):
            longitude = UNSET
        else:
            longitude = self.longitude

        phone_number: None | str | Unset
        if isinstance(self.phone_number, Unset):
            phone_number = UNSET
        else:
            phone_number = self.phone_number

        timezone: None | str | Unset
        if isinstance(self.timezone, Unset):
            timezone = UNSET
        else:
            timezone = self.timezone

        emergency_contact_name: None | str | Unset
        if isinstance(self.emergency_contact_name, Unset):
            emergency_contact_name = UNSET
        else:
            emergency_contact_name = self.emergency_contact_name

        emergency_contact_phone: None | str | Unset
        if isinstance(self.emergency_contact_phone, Unset):
            emergency_contact_phone = UNSET
        else:
            emergency_contact_phone = self.emergency_contact_phone

        e911_callback_number: None | str | Unset
        if isinstance(self.e911_callback_number, Unset):
            e911_callback_number = UNSET
        else:
            e911_callback_number = self.e911_callback_number

        civic: dict[str, Any] | None | Unset
        if isinstance(self.civic, Unset):
            civic = UNSET
        elif isinstance(self.civic, CivicElements):
            civic = self.civic.to_dict()
        else:
            civic = self.civic


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "name": name,
        })
        if tenant_id is not UNSET:
            field_dict["tenant_id"] = tenant_id
        if description is not UNSET:
            field_dict["description"] = description
        if address_line1 is not UNSET:
            field_dict["address_line1"] = address_line1
        if address_line2 is not UNSET:
            field_dict["address_line2"] = address_line2
        if city is not UNSET:
            field_dict["city"] = city
        if state is not UNSET:
            field_dict["state"] = state
        if postal_code is not UNSET:
            field_dict["postal_code"] = postal_code
        if country is not UNSET:
            field_dict["country"] = country
        if floor is not UNSET:
            field_dict["floor"] = floor
        if building is not UNSET:
            field_dict["building"] = building
        if latitude is not UNSET:
            field_dict["latitude"] = latitude
        if longitude is not UNSET:
            field_dict["longitude"] = longitude
        if phone_number is not UNSET:
            field_dict["phone_number"] = phone_number
        if timezone is not UNSET:
            field_dict["timezone"] = timezone
        if emergency_contact_name is not UNSET:
            field_dict["emergency_contact_name"] = emergency_contact_name
        if emergency_contact_phone is not UNSET:
            field_dict["emergency_contact_phone"] = emergency_contact_phone
        if e911_callback_number is not UNSET:
            field_dict["e911_callback_number"] = e911_callback_number
        if civic is not UNSET:
            field_dict["civic"] = civic

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.civic_elements import CivicElements # noqa: PLC0415
        d = dict(src_dict)
        name = d.pop("name")

        tenant_id = d.pop("tenant_id", UNSET)

        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        def _parse_address_line1(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        address_line1 = _parse_address_line1(d.pop("address_line1", UNSET))


        def _parse_address_line2(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        address_line2 = _parse_address_line2(d.pop("address_line2", UNSET))


        def _parse_city(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        city = _parse_city(d.pop("city", UNSET))


        def _parse_state(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        state = _parse_state(d.pop("state", UNSET))


        def _parse_postal_code(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        postal_code = _parse_postal_code(d.pop("postal_code", UNSET))


        def _parse_country(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        country = _parse_country(d.pop("country", UNSET))


        def _parse_floor(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        floor = _parse_floor(d.pop("floor", UNSET))


        def _parse_building(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        building = _parse_building(d.pop("building", UNSET))


        def _parse_latitude(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        latitude = _parse_latitude(d.pop("latitude", UNSET))


        def _parse_longitude(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        longitude = _parse_longitude(d.pop("longitude", UNSET))


        def _parse_phone_number(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        phone_number = _parse_phone_number(d.pop("phone_number", UNSET))


        def _parse_timezone(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        timezone = _parse_timezone(d.pop("timezone", UNSET))


        def _parse_emergency_contact_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        emergency_contact_name = _parse_emergency_contact_name(d.pop("emergency_contact_name", UNSET))


        def _parse_emergency_contact_phone(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        emergency_contact_phone = _parse_emergency_contact_phone(d.pop("emergency_contact_phone", UNSET))


        def _parse_e911_callback_number(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        e911_callback_number = _parse_e911_callback_number(d.pop("e911_callback_number", UNSET))


        def _parse_civic(data: object) -> CivicElements | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                civic_type_0 = CivicElements.from_dict(data)



                return civic_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(CivicElements | None | Unset, data)

        civic = _parse_civic(d.pop("civic", UNSET))


        create_location_request = cls(
            name=name,
            tenant_id=tenant_id,
            description=description,
            address_line1=address_line1,
            address_line2=address_line2,
            city=city,
            state=state,
            postal_code=postal_code,
            country=country,
            floor=floor,
            building=building,
            latitude=latitude,
            longitude=longitude,
            phone_number=phone_number,
            timezone=timezone,
            emergency_contact_name=emergency_contact_name,
            emergency_contact_phone=emergency_contact_phone,
            e911_callback_number=e911_callback_number,
            civic=civic,
        )


        create_location_request.additional_properties = d
        return create_location_request

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
