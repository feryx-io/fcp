from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.civic_elements import CivicElements
  from ..models.contact_response import ContactResponse





T = TypeVar("T", bound="PageLocationResponseDataItem")



@_attrs_define
class PageLocationResponseDataItem:
    """ Location response.

        Attributes:
            id (str):
            tenant_id (str):
            name (str):
            emergency_contacts (list[ContactResponse]): Who is told about an emergency call from this site, before the
                tenant's contacts (`docs/E911.md` "On-site notification").
            notify_tenant_contacts (bool): Whether the tenant's contacts are told too.
            civic (CivicElements): The RFC 5139 civic address elements beyond city, state, postal code and
                country, which [`E911Address`] carries as fields of its own.
            e911_validated (bool): Whether a validator has confirmed the address
                (`POST /locations/:id/e911/validate`); editing it clears this.
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
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
            e911_callback_number (None | str | Unset):
            e911_validated_at (datetime.datetime | None | Unset):
            e911_validated_by (None | str | Unset):
            e911_validation_reference (None | str | Unset):
     """

    id: str
    tenant_id: str
    name: str
    emergency_contacts: list[ContactResponse]
    notify_tenant_contacts: bool
    civic: CivicElements
    e911_validated: bool
    created_at: datetime.datetime
    updated_at: datetime.datetime
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
    e911_callback_number: None | str | Unset = UNSET
    e911_validated_at: datetime.datetime | None | Unset = UNSET
    e911_validated_by: None | str | Unset = UNSET
    e911_validation_reference: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.civic_elements import CivicElements # noqa: PLC0415
        from ..models.contact_response import ContactResponse # noqa: PLC0415
        id = self.id

        tenant_id = self.tenant_id

        name = self.name

        emergency_contacts = []
        for emergency_contacts_item_data in self.emergency_contacts:
            emergency_contacts_item = emergency_contacts_item_data.to_dict()
            emergency_contacts.append(emergency_contacts_item)



        notify_tenant_contacts = self.notify_tenant_contacts

        civic = self.civic.to_dict()

        e911_validated = self.e911_validated

        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

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

        e911_callback_number: None | str | Unset
        if isinstance(self.e911_callback_number, Unset):
            e911_callback_number = UNSET
        else:
            e911_callback_number = self.e911_callback_number

        e911_validated_at: None | str | Unset
        if isinstance(self.e911_validated_at, Unset):
            e911_validated_at = UNSET
        elif isinstance(self.e911_validated_at, datetime.datetime):
            e911_validated_at = self.e911_validated_at.isoformat()
        else:
            e911_validated_at = self.e911_validated_at

        e911_validated_by: None | str | Unset
        if isinstance(self.e911_validated_by, Unset):
            e911_validated_by = UNSET
        else:
            e911_validated_by = self.e911_validated_by

        e911_validation_reference: None | str | Unset
        if isinstance(self.e911_validation_reference, Unset):
            e911_validation_reference = UNSET
        else:
            e911_validation_reference = self.e911_validation_reference


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "name": name,
            "emergency_contacts": emergency_contacts,
            "notify_tenant_contacts": notify_tenant_contacts,
            "civic": civic,
            "e911_validated": e911_validated,
            "created_at": created_at,
            "updated_at": updated_at,
        })
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
        if e911_callback_number is not UNSET:
            field_dict["e911_callback_number"] = e911_callback_number
        if e911_validated_at is not UNSET:
            field_dict["e911_validated_at"] = e911_validated_at
        if e911_validated_by is not UNSET:
            field_dict["e911_validated_by"] = e911_validated_by
        if e911_validation_reference is not UNSET:
            field_dict["e911_validation_reference"] = e911_validation_reference

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.civic_elements import CivicElements # noqa: PLC0415
        from ..models.contact_response import ContactResponse # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        name = d.pop("name")

        emergency_contacts = []
        _emergency_contacts = d.pop("emergency_contacts")
        for emergency_contacts_item_data in (_emergency_contacts):
            emergency_contacts_item = ContactResponse.from_dict(emergency_contacts_item_data)



            emergency_contacts.append(emergency_contacts_item)


        notify_tenant_contacts = d.pop("notify_tenant_contacts")

        civic = CivicElements.from_dict(d.pop("civic"))




        e911_validated = d.pop("e911_validated")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




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


        def _parse_e911_callback_number(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        e911_callback_number = _parse_e911_callback_number(d.pop("e911_callback_number", UNSET))


        def _parse_e911_validated_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                e911_validated_at_type_0 = datetime.datetime.fromisoformat(data)



                return e911_validated_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        e911_validated_at = _parse_e911_validated_at(d.pop("e911_validated_at", UNSET))


        def _parse_e911_validated_by(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        e911_validated_by = _parse_e911_validated_by(d.pop("e911_validated_by", UNSET))


        def _parse_e911_validation_reference(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        e911_validation_reference = _parse_e911_validation_reference(d.pop("e911_validation_reference", UNSET))


        page_location_response_data_item = cls(
            id=id,
            tenant_id=tenant_id,
            name=name,
            emergency_contacts=emergency_contacts,
            notify_tenant_contacts=notify_tenant_contacts,
            civic=civic,
            e911_validated=e911_validated,
            created_at=created_at,
            updated_at=updated_at,
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
            e911_callback_number=e911_callback_number,
            e911_validated_at=e911_validated_at,
            e911_validated_by=e911_validated_by,
            e911_validation_reference=e911_validation_reference,
        )


        page_location_response_data_item.additional_properties = d
        return page_location_response_data_item

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
