from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.target_view_type_0_type import TargetViewType0Type
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="TargetViewType0")



@_attrs_define
class TargetViewType0:
    """ 
        Attributes:
            host (str):
            port (int):
            user (str):
            path (str):
            has_private_key (bool):
            type_ (TargetViewType0Type):
            host_key (None | str | Unset): The server's host key as pinned by the first test.
     """

    host: str
    port: int
    user: str
    path: str
    has_private_key: bool
    type_: TargetViewType0Type
    host_key: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        host = self.host

        port = self.port

        user = self.user

        path = self.path

        has_private_key = self.has_private_key

        type_ = self.type_.value

        host_key: None | str | Unset
        if isinstance(self.host_key, Unset):
            host_key = UNSET
        else:
            host_key = self.host_key


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "host": host,
            "port": port,
            "user": user,
            "path": path,
            "has_private_key": has_private_key,
            "type": type_,
        })
        if host_key is not UNSET:
            field_dict["host_key"] = host_key

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        host = d.pop("host")

        port = d.pop("port")

        user = d.pop("user")

        path = d.pop("path")

        has_private_key = d.pop("has_private_key")

        type_ = TargetViewType0Type(d.pop("type"))




        def _parse_host_key(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        host_key = _parse_host_key(d.pop("host_key", UNSET))


        target_view_type_0 = cls(
            host=host,
            port=port,
            user=user,
            path=path,
            has_private_key=has_private_key,
            type_=type_,
            host_key=host_key,
        )


        target_view_type_0.additional_properties = d
        return target_view_type_0

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
