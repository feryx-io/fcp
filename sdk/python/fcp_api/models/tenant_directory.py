from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset






T = TypeVar("T", bound="TenantDirectory")



@_attrs_define
class TenantDirectory:
    """ What a tenant's people see of each other through `/me/directory`.

        Attributes:
            visible_to_users (bool | Unset): Whether its users may list the tenant's users, extensions and
                presence; on unless a tenant turns it off.
     """

    visible_to_users: bool | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        visible_to_users = self.visible_to_users


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if visible_to_users is not UNSET:
            field_dict["visible_to_users"] = visible_to_users

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        visible_to_users = d.pop("visible_to_users", UNSET)

        tenant_directory = cls(
            visible_to_users=visible_to_users,
        )


        tenant_directory.additional_properties = d
        return tenant_directory

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
