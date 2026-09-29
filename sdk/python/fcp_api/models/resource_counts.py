from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="ResourceCounts")



@_attrs_define
class ResourceCounts:
    """ Counts of provisioned resources for the tenant.

        Attributes:
            phone_numbers (int):
            users (int):
            devices (int):
     """

    phone_numbers: int
    users: int
    devices: int
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        phone_numbers = self.phone_numbers

        users = self.users

        devices = self.devices


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "phone_numbers": phone_numbers,
            "users": users,
            "devices": devices,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        phone_numbers = d.pop("phone_numbers")

        users = d.pop("users")

        devices = d.pop("devices")

        resource_counts = cls(
            phone_numbers=phone_numbers,
            users=users,
            devices=devices,
        )


        resource_counts.additional_properties = d
        return resource_counts

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
