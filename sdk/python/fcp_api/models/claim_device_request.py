from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="ClaimDeviceRequest")



@_attrs_define
class ClaimDeviceRequest:
    """ What a claimed or imported phone becomes.

        Attributes:
            name (None | str | Unset): Unset: its vendor, model and MAC.
            device_type (None | str | Unset): Unset: `desk_phone`.
            owner_user_id (None | str | Unset): Its owner, by id or by username.
            owner_username (None | str | Unset):
            config_template_id (None | str | Unset): Unset: picked by vendor and model when the phone fetches.
            location_id (None | str | Unset):
     """

    name: None | str | Unset = UNSET
    device_type: None | str | Unset = UNSET
    owner_user_id: None | str | Unset = UNSET
    owner_username: None | str | Unset = UNSET
    config_template_id: None | str | Unset = UNSET
    location_id: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        name: None | str | Unset
        if isinstance(self.name, Unset):
            name = UNSET
        else:
            name = self.name

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
        })
        if name is not UNSET:
            field_dict["name"] = name
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
        def _parse_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        name = _parse_name(d.pop("name", UNSET))


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


        claim_device_request = cls(
            name=name,
            device_type=device_type,
            owner_user_id=owner_user_id,
            owner_username=owner_username,
            config_template_id=config_template_id,
            location_id=location_id,
        )


        claim_device_request.additional_properties = d
        return claim_device_request

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
