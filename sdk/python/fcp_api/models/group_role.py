from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="GroupRole")



@_attrs_define
class GroupRole:
    """ A provider group, and the role its members get.

        Attributes:
            group (str):
            role (str): Any role but `super_admin`, which no tenant's provider grants.
     """

    group: str
    role: str





    def to_dict(self) -> dict[str, Any]:
        group = self.group

        role = self.role


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "group": group,
            "role": role,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        group = d.pop("group")

        role = d.pop("role")

        group_role = cls(
            group=group,
            role=role,
        )

        return group_role

