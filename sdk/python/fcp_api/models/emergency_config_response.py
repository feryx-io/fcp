from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.contact_response import ContactResponse





T = TypeVar("T", bound="EmergencyConfigResponse")



@_attrs_define
class EmergencyConfigResponse:
    """ 
        Attributes:
            tenant_id (str):
            emergency_numbers (list[str]):
            test_numbers (list[str]):
            dial_prefixes (list[str]):
            contacts (list[ContactResponse]): Who is told when an emergency call is placed, typed by channel
                (`docs/E911_BLOCK_2.md` §8.1).
            notify_contacts (list[str]): The same contacts as the plain strings this field has always
                carried, for clients written before block 2.
            require_dispatchable_location (bool):
            trust_device_locations (list[str]): Device types whose own location (a PIDF-LO they send with the call)
                this tenant trusts.
            declared_location_ttl_secs (int): How long a user's declared location stands, in seconds.
            configured (bool): False when the tenant has never saved a config (defaults shown).
            updated_at (datetime.datetime):
     """

    tenant_id: str
    emergency_numbers: list[str]
    test_numbers: list[str]
    dial_prefixes: list[str]
    contacts: list[ContactResponse]
    notify_contacts: list[str]
    require_dispatchable_location: bool
    trust_device_locations: list[str]
    declared_location_ttl_secs: int
    configured: bool
    updated_at: datetime.datetime
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.contact_response import ContactResponse # noqa: PLC0415
        tenant_id = self.tenant_id

        emergency_numbers = self.emergency_numbers



        test_numbers = self.test_numbers



        dial_prefixes = self.dial_prefixes



        contacts = []
        for contacts_item_data in self.contacts:
            contacts_item = contacts_item_data.to_dict()
            contacts.append(contacts_item)



        notify_contacts = self.notify_contacts



        require_dispatchable_location = self.require_dispatchable_location

        trust_device_locations = self.trust_device_locations



        declared_location_ttl_secs = self.declared_location_ttl_secs

        configured = self.configured

        updated_at = self.updated_at.isoformat()


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "tenant_id": tenant_id,
            "emergency_numbers": emergency_numbers,
            "test_numbers": test_numbers,
            "dial_prefixes": dial_prefixes,
            "contacts": contacts,
            "notify_contacts": notify_contacts,
            "require_dispatchable_location": require_dispatchable_location,
            "trust_device_locations": trust_device_locations,
            "declared_location_ttl_secs": declared_location_ttl_secs,
            "configured": configured,
            "updated_at": updated_at,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.contact_response import ContactResponse # noqa: PLC0415
        d = dict(src_dict)
        tenant_id = d.pop("tenant_id")

        emergency_numbers = cast(list[str], d.pop("emergency_numbers"))


        test_numbers = cast(list[str], d.pop("test_numbers"))


        dial_prefixes = cast(list[str], d.pop("dial_prefixes"))


        contacts = []
        _contacts = d.pop("contacts")
        for contacts_item_data in (_contacts):
            contacts_item = ContactResponse.from_dict(contacts_item_data)



            contacts.append(contacts_item)


        notify_contacts = cast(list[str], d.pop("notify_contacts"))


        require_dispatchable_location = d.pop("require_dispatchable_location")

        trust_device_locations = cast(list[str], d.pop("trust_device_locations"))


        declared_location_ttl_secs = d.pop("declared_location_ttl_secs")

        configured = d.pop("configured")

        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        emergency_config_response = cls(
            tenant_id=tenant_id,
            emergency_numbers=emergency_numbers,
            test_numbers=test_numbers,
            dial_prefixes=dial_prefixes,
            contacts=contacts,
            notify_contacts=notify_contacts,
            require_dispatchable_location=require_dispatchable_location,
            trust_device_locations=trust_device_locations,
            declared_location_ttl_secs=declared_location_ttl_secs,
            configured=configured,
            updated_at=updated_at,
        )


        emergency_config_response.additional_properties = d
        return emergency_config_response

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
