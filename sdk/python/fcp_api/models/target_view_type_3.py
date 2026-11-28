from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.target_view_type_3_type import TargetViewType3Type
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="TargetViewType3")



@_attrs_define
class TargetViewType3:
    """ A restic REST server.

        Attributes:
            url (str):
            has_password (bool):
            type_ (TargetViewType3Type):
            username (None | str | Unset):
     """

    url: str
    has_password: bool
    type_: TargetViewType3Type
    username: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        url = self.url

        has_password = self.has_password

        type_ = self.type_.value

        username: None | str | Unset
        if isinstance(self.username, Unset):
            username = UNSET
        else:
            username = self.username


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "url": url,
            "has_password": has_password,
            "type": type_,
        })
        if username is not UNSET:
            field_dict["username"] = username

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        url = d.pop("url")

        has_password = d.pop("has_password")

        type_ = TargetViewType3Type(d.pop("type"))




        def _parse_username(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        username = _parse_username(d.pop("username", UNSET))


        target_view_type_3 = cls(
            url=url,
            has_password=has_password,
            type_=type_,
            username=username,
        )


        target_view_type_3.additional_properties = d
        return target_view_type_3

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
