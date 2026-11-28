from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.target_request_type_0_type import TargetRequestType0Type
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="TargetRequestType0")



@_attrs_define
class TargetRequestType0:
    """ 
        Attributes:
            host (str):
            user (str):
            path (str):
            type_ (TargetRequestType0Type):
            port (int | None | Unset):
            private_key (None | str | Unset): An OpenSSH private key; write-only.
     """

    host: str
    user: str
    path: str
    type_: TargetRequestType0Type
    port: int | None | Unset = UNSET
    private_key: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        host = self.host

        user = self.user

        path = self.path

        type_ = self.type_.value

        port: int | None | Unset
        if isinstance(self.port, Unset):
            port = UNSET
        else:
            port = self.port

        private_key: None | str | Unset
        if isinstance(self.private_key, Unset):
            private_key = UNSET
        else:
            private_key = self.private_key


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "host": host,
            "user": user,
            "path": path,
            "type": type_,
        })
        if port is not UNSET:
            field_dict["port"] = port
        if private_key is not UNSET:
            field_dict["private_key"] = private_key

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        host = d.pop("host")

        user = d.pop("user")

        path = d.pop("path")

        type_ = TargetRequestType0Type(d.pop("type"))




        def _parse_port(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        port = _parse_port(d.pop("port", UNSET))


        def _parse_private_key(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        private_key = _parse_private_key(d.pop("private_key", UNSET))


        target_request_type_0 = cls(
            host=host,
            user=user,
            path=path,
            type_=type_,
            port=port,
            private_key=private_key,
        )


        target_request_type_0.additional_properties = d
        return target_request_type_0

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
