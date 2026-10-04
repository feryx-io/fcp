from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="ImportDeviceRow")



@_attrs_define
class ImportDeviceRow:
    """ One phone to import.

        Attributes:
            mac_address (str):
            name (None | str | Unset):
            model (None | str | Unset):
            vendor (None | str | Unset): Unset: read from the MAC's prefix where it is a vendor's.
            device_type (None | str | Unset):
            owner_user_id (None | str | Unset):
            owner_username (None | str | Unset):
            config_template_id (None | str | Unset):
            location_id (None | str | Unset):
     """

    mac_address: str
    name: None | str | Unset = UNSET
    model: None | str | Unset = UNSET
    vendor: None | str | Unset = UNSET
    device_type: None | str | Unset = UNSET
    owner_user_id: None | str | Unset = UNSET
    owner_username: None | str | Unset = UNSET
    config_template_id: None | str | Unset = UNSET
    location_id: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        mac_address = self.mac_address

        name: None | str | Unset
        if isinstance(self.name, Unset):
            name = UNSET
        else:
            name = self.name

        model: None | str | Unset
        if isinstance(self.model, Unset):
            model = UNSET
        else:
            model = self.model

        vendor: None | str | Unset
        if isinstance(self.vendor, Unset):
            vendor = UNSET
        else:
            vendor = self.vendor

        device_type: None | str | Unset
        if isinstance(self.device_type, Unset):
            device_type = UNSET
        else:
            device_type = self.device_type

        owner_user_id: None | str | Unset
        if isinstance(self.owner_user_id, Unset):
            owner_user_id = UNSET
        else:
            owner_user_id = self.owner_user_id

        owner_username: None | str | Unset
        if isinstance(self.owner_username, Unset):
            owner_username = UNSET
        else:
            owner_username = self.owner_username

        config_template_id: None | str | Unset
        if isinstance(self.config_template_id, Unset):
            config_template_id = UNSET
        else:
            config_template_id = self.config_template_id

        location_id: None | str | Unset
        if isinstance(self.location_id, Unset):
            location_id = UNSET
        else:
            location_id = self.location_id


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "mac_address": mac_address,
        })
        if name is not UNSET:
            field_dict["name"] = name
        if model is not UNSET:
            field_dict["model"] = model
        if vendor is not UNSET:
            field_dict["vendor"] = vendor
        if device_type is not UNSET:
            field_dict["device_type"] = device_type
        if owner_user_id is not UNSET:
            field_dict["owner_user_id"] = owner_user_id
        if owner_username is not UNSET:
            field_dict["owner_username"] = owner_username
        if config_template_id is not UNSET:
            field_dict["config_template_id"] = config_template_id
        if location_id is not UNSET:
            field_dict["location_id"] = location_id

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        mac_address = d.pop("mac_address")

        def _parse_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        name = _parse_name(d.pop("name", UNSET))


        def _parse_model(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        model = _parse_model(d.pop("model", UNSET))


        def _parse_vendor(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        vendor = _parse_vendor(d.pop("vendor", UNSET))


        def _parse_device_type(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        device_type = _parse_device_type(d.pop("device_type", UNSET))


        def _parse_owner_user_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        owner_user_id = _parse_owner_user_id(d.pop("owner_user_id", UNSET))


        def _parse_owner_username(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        owner_username = _parse_owner_username(d.pop("owner_username", UNSET))


        def _parse_config_template_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        config_template_id = _parse_config_template_id(d.pop("config_template_id", UNSET))


        def _parse_location_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        location_id = _parse_location_id(d.pop("location_id", UNSET))


        import_device_row = cls(
            mac_address=mac_address,
            name=name,
            model=model,
            vendor=vendor,
            device_type=device_type,
            owner_user_id=owner_user_id,
            owner_username=owner_username,
            config_template_id=config_template_id,
            location_id=location_id,
        )


        import_device_row.additional_properties = d
        return import_device_row

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
