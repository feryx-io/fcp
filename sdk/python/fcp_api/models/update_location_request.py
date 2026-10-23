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
  from ..models.contact_input_type_1 import ContactInputType1





T = TypeVar("T", bound="UpdateLocationRequest")



@_attrs_define
class UpdateLocationRequest:
    """ Update location request.

        Attributes:
            name (None | str | Unset):
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
            emergency_contacts (list[ContactInputType1 | str] | None | Unset): Replaces the site's emergency contacts when
                given.
            notify_tenant_contacts (bool | None | Unset):
            e911_callback_number (None | str | Unset):
            civic (CivicElements | None | Unset):
     """

    name: None | str | Unset = UNSET
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
    emergency_contacts: list[ContactInputType1 | str] | None | Unset = UNSET
    notify_tenant_contacts: bool | None | Unset = UNSET
    e911_callback_number: None | str | Unset = UNSET
    civic: CivicElements | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.civic_elements import CivicElements # noqa: PLC0415
        from ..models.contact_input_type_1 import ContactInputType1 # noqa: PLC0415
        name: None | str | Unset
        if isinstance(self.name, Unset):
            name = UNSET
        else:
            name = self.name

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

        emergency_contacts: list[dict[str, Any] | str] | None | Unset
        if isinstance(self.emergency_contacts, Unset):
            emergency_contacts = UNSET
        elif isinstance(self.emergency_contacts, list):
            emergency_contacts = []
            for emergency_contacts_type_0_item_data in self.emergency_contacts:
                emergency_contacts_type_0_item: dict[str, Any] | str
                if isinstance(emergency_contacts_type_0_item_data, ContactInputType1):
                    emergency_contacts_type_0_item = emergency_contacts_type_0_item_data.to_dict()
                else:
                    emergency_contacts_type_0_item = emergency_contacts_type_0_item_data
                emergency_contacts.append(emergency_contacts_type_0_item)


        else:
            emergency_contacts = self.emergency_contacts

        notify_tenant_contacts: bool | None | Unset
        if isinstance(self.notify_tenant_contacts, Unset):
            notify_tenant_contacts = UNSET
        else:
            notify_tenant_contacts = self.notify_tenant_contacts

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
        })
        if name is not UNSET:
            field_dict["name"] = name
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
        if emergency_contacts is not UNSET:
            field_dict["emergency_contacts"] = emergency_contacts
        if notify_tenant_contacts is not UNSET:
            field_dict["notify_tenant_contacts"] = notify_tenant_contacts
        if e911_callback_number is not UNSET:
            field_dict["e911_callback_number"] = e911_callback_number
        if civic is not UNSET:
            field_dict["civic"] = civic

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.civic_elements import CivicElements # noqa: PLC0415
        from ..models.contact_input_type_1 import ContactInputType1 # noqa: PLC0415
        d = dict(src_dict)
        def _parse_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        name = _parse_name(d.pop("name", UNSET))


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


        def _parse_emergency_contacts(data: object) -> list[ContactInputType1 | str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                emergency_contacts_type_0 = []
                _emergency_contacts_type_0 = data
                for emergency_contacts_type_0_item_data in (_emergency_contacts_type_0):
                    def _parse_emergency_contacts_type_0_item(data: object) -> ContactInputType1 | str:
                        try:
                            if not isinstance(data, dict):
                                raise TypeError()
                            componentsschemas_contact_input_type_1 = ContactInputType1.from_dict(data)



                            return componentsschemas_contact_input_type_1
                        except (TypeError, ValueError, AttributeError, KeyError):
                            pass
                        return cast(ContactInputType1 | str, data)

                    emergency_contacts_type_0_item = _parse_emergency_contacts_type_0_item(emergency_contacts_type_0_item_data)

                    emergency_contacts_type_0.append(emergency_contacts_type_0_item)

                return emergency_contacts_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[ContactInputType1 | str] | None | Unset, data)

        emergency_contacts = _parse_emergency_contacts(d.pop("emergency_contacts", UNSET))


        def _parse_notify_tenant_contacts(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        notify_tenant_contacts = _parse_notify_tenant_contacts(d.pop("notify_tenant_contacts", UNSET))


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


        update_location_request = cls(
            name=name,
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
            emergency_contacts=emergency_contacts,
            notify_tenant_contacts=notify_tenant_contacts,
            e911_callback_number=e911_callback_number,
            civic=civic,
        )


        update_location_request.additional_properties = d
        return update_location_request

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
