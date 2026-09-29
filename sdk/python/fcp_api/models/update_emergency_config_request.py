from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.contact_input_type_1 import ContactInputType1





T = TypeVar("T", bound="UpdateEmergencyConfigRequest")



@_attrs_define
class UpdateEmergencyConfigRequest:
    """ 
        Attributes:
            emergency_numbers (list[str] | None | Unset):
            test_numbers (list[str] | None | Unset):
            dial_prefixes (list[str] | None | Unset):
            contacts (list[ContactInputType1 | str] | None | Unset): Replaces the notification contacts. Strings are read by
                shape.
            notify_contacts (list[ContactInputType1 | str] | None | Unset): The field this took before block 2; `contacts`
                wins when both are
                given.
            require_dispatchable_location (bool | None | Unset):
            trust_device_locations (list[str] | None | Unset): Device types trusted to send their own location (`softphone`,
                `desk_phone`, `webrtc`, `mobile`, …); replaces the list.
            declared_location_ttl_secs (int | None | Unset): How long a declared location stands, in seconds (60 to 30
                days).
     """

    emergency_numbers: list[str] | None | Unset = UNSET
    test_numbers: list[str] | None | Unset = UNSET
    dial_prefixes: list[str] | None | Unset = UNSET
    contacts: list[ContactInputType1 | str] | None | Unset = UNSET
    notify_contacts: list[ContactInputType1 | str] | None | Unset = UNSET
    require_dispatchable_location: bool | None | Unset = UNSET
    trust_device_locations: list[str] | None | Unset = UNSET
    declared_location_ttl_secs: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.contact_input_type_1 import ContactInputType1 # noqa: PLC0415
        emergency_numbers: list[str] | None | Unset
        if isinstance(self.emergency_numbers, Unset):
            emergency_numbers = UNSET
        elif isinstance(self.emergency_numbers, list):
            emergency_numbers = self.emergency_numbers


        else:
            emergency_numbers = self.emergency_numbers

        test_numbers: list[str] | None | Unset
        if isinstance(self.test_numbers, Unset):
            test_numbers = UNSET
        elif isinstance(self.test_numbers, list):
            test_numbers = self.test_numbers


        else:
            test_numbers = self.test_numbers

        dial_prefixes: list[str] | None | Unset
        if isinstance(self.dial_prefixes, Unset):
            dial_prefixes = UNSET
        elif isinstance(self.dial_prefixes, list):
            dial_prefixes = self.dial_prefixes


        else:
            dial_prefixes = self.dial_prefixes

        contacts: list[dict[str, Any] | str] | None | Unset
        if isinstance(self.contacts, Unset):
            contacts = UNSET
        elif isinstance(self.contacts, list):
            contacts = []
            for contacts_type_0_item_data in self.contacts:
                contacts_type_0_item: dict[str, Any] | str
                if isinstance(contacts_type_0_item_data, ContactInputType1):
                    contacts_type_0_item = contacts_type_0_item_data.to_dict()
                else:
                    contacts_type_0_item = contacts_type_0_item_data
                contacts.append(contacts_type_0_item)


        else:
            contacts = self.contacts

        notify_contacts: list[dict[str, Any] | str] | None | Unset
        if isinstance(self.notify_contacts, Unset):
            notify_contacts = UNSET
        elif isinstance(self.notify_contacts, list):
            notify_contacts = []
            for notify_contacts_type_0_item_data in self.notify_contacts:
                notify_contacts_type_0_item: dict[str, Any] | str
                if isinstance(notify_contacts_type_0_item_data, ContactInputType1):
                    notify_contacts_type_0_item = notify_contacts_type_0_item_data.to_dict()
                else:
                    notify_contacts_type_0_item = notify_contacts_type_0_item_data
                notify_contacts.append(notify_contacts_type_0_item)


        else:
            notify_contacts = self.notify_contacts

        require_dispatchable_location: bool | None | Unset
        if isinstance(self.require_dispatchable_location, Unset):
            require_dispatchable_location = UNSET
        else:
            require_dispatchable_location = self.require_dispatchable_location

        trust_device_locations: list[str] | None | Unset
        if isinstance(self.trust_device_locations, Unset):
            trust_device_locations = UNSET
        elif isinstance(self.trust_device_locations, list):
            trust_device_locations = self.trust_device_locations


        else:
            trust_device_locations = self.trust_device_locations

        declared_location_ttl_secs: int | None | Unset
        if isinstance(self.declared_location_ttl_secs, Unset):
            declared_location_ttl_secs = UNSET
        else:
            declared_location_ttl_secs = self.declared_location_ttl_secs


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if emergency_numbers is not UNSET:
            field_dict["emergency_numbers"] = emergency_numbers
        if test_numbers is not UNSET:
            field_dict["test_numbers"] = test_numbers
        if dial_prefixes is not UNSET:
            field_dict["dial_prefixes"] = dial_prefixes
        if contacts is not UNSET:
            field_dict["contacts"] = contacts
        if notify_contacts is not UNSET:
            field_dict["notify_contacts"] = notify_contacts
        if require_dispatchable_location is not UNSET:
            field_dict["require_dispatchable_location"] = require_dispatchable_location
        if trust_device_locations is not UNSET:
            field_dict["trust_device_locations"] = trust_device_locations
        if declared_location_ttl_secs is not UNSET:
            field_dict["declared_location_ttl_secs"] = declared_location_ttl_secs

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.contact_input_type_1 import ContactInputType1 # noqa: PLC0415
        d = dict(src_dict)
        def _parse_emergency_numbers(data: object) -> list[str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                emergency_numbers_type_0 = cast(list[str], data)

                return emergency_numbers_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[str] | None | Unset, data)

        emergency_numbers = _parse_emergency_numbers(d.pop("emergency_numbers", UNSET))


        def _parse_test_numbers(data: object) -> list[str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                test_numbers_type_0 = cast(list[str], data)

                return test_numbers_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[str] | None | Unset, data)

        test_numbers = _parse_test_numbers(d.pop("test_numbers", UNSET))


        def _parse_dial_prefixes(data: object) -> list[str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                dial_prefixes_type_0 = cast(list[str], data)

                return dial_prefixes_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[str] | None | Unset, data)

        dial_prefixes = _parse_dial_prefixes(d.pop("dial_prefixes", UNSET))


        def _parse_contacts(data: object) -> list[ContactInputType1 | str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                contacts_type_0 = []
                _contacts_type_0 = data
                for contacts_type_0_item_data in (_contacts_type_0):
                    def _parse_contacts_type_0_item(data: object) -> ContactInputType1 | str:
                        try:
                            if not isinstance(data, dict):
                                raise TypeError()
                            componentsschemas_contact_input_type_1 = ContactInputType1.from_dict(data)



                            return componentsschemas_contact_input_type_1
                        except (TypeError, ValueError, AttributeError, KeyError):
                            pass
                        return cast(ContactInputType1 | str, data)

                    contacts_type_0_item = _parse_contacts_type_0_item(contacts_type_0_item_data)

                    contacts_type_0.append(contacts_type_0_item)

                return contacts_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[ContactInputType1 | str] | None | Unset, data)

        contacts = _parse_contacts(d.pop("contacts", UNSET))


        def _parse_notify_contacts(data: object) -> list[ContactInputType1 | str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                notify_contacts_type_0 = []
                _notify_contacts_type_0 = data
                for notify_contacts_type_0_item_data in (_notify_contacts_type_0):
                    def _parse_notify_contacts_type_0_item(data: object) -> ContactInputType1 | str:
                        try:
                            if not isinstance(data, dict):
                                raise TypeError()
                            componentsschemas_contact_input_type_1 = ContactInputType1.from_dict(data)



                            return componentsschemas_contact_input_type_1
                        except (TypeError, ValueError, AttributeError, KeyError):
                            pass
                        return cast(ContactInputType1 | str, data)

                    notify_contacts_type_0_item = _parse_notify_contacts_type_0_item(notify_contacts_type_0_item_data)

                    notify_contacts_type_0.append(notify_contacts_type_0_item)

                return notify_contacts_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[ContactInputType1 | str] | None | Unset, data)

        notify_contacts = _parse_notify_contacts(d.pop("notify_contacts", UNSET))


        def _parse_require_dispatchable_location(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        require_dispatchable_location = _parse_require_dispatchable_location(d.pop("require_dispatchable_location", UNSET))


        def _parse_trust_device_locations(data: object) -> list[str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                trust_device_locations_type_0 = cast(list[str], data)

                return trust_device_locations_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[str] | None | Unset, data)

        trust_device_locations = _parse_trust_device_locations(d.pop("trust_device_locations", UNSET))


        def _parse_declared_location_ttl_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        declared_location_ttl_secs = _parse_declared_location_ttl_secs(d.pop("declared_location_ttl_secs", UNSET))


        update_emergency_config_request = cls(
            emergency_numbers=emergency_numbers,
            test_numbers=test_numbers,
            dial_prefixes=dial_prefixes,
            contacts=contacts,
            notify_contacts=notify_contacts,
            require_dispatchable_location=require_dispatchable_location,
            trust_device_locations=trust_device_locations,
            declared_location_ttl_secs=declared_location_ttl_secs,
        )


        update_emergency_config_request.additional_properties = d
        return update_emergency_config_request

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
