from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.target_view_type_4_type import TargetViewType4Type






T = TypeVar("T", bound="TargetViewType4")



@_attrs_define
class TargetViewType4:
    """ An rclone remote.

        Attributes:
            remote (str):
            has_config (bool):
            type_ (TargetViewType4Type):
     """

    remote: str
    has_config: bool
    type_: TargetViewType4Type
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        remote = self.remote

        has_config = self.has_config

        type_ = self.type_.value


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "remote": remote,
            "has_config": has_config,
            "type": type_,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        remote = d.pop("remote")

        has_config = d.pop("has_config")

        type_ = TargetViewType4Type(d.pop("type"))




        target_view_type_4 = cls(
            remote=remote,
            has_config=has_config,
            type_=type_,
        )


        target_view_type_4.additional_properties = d
        return target_view_type_4

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
