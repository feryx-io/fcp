from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="SoftphonePassword")



@_attrs_define
class SoftphonePassword:
    """ A new SIP password for the app's device.

        Attributes:
            sip_username (str):
            sip_password (str): Shown here once.
            realm (str):
     """

    sip_username: str
    sip_password: str
    realm: str
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        sip_username = self.sip_username

        sip_password = self.sip_password

        realm = self.realm


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "sip_username": sip_username,
            "sip_password": sip_password,
            "realm": realm,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        sip_username = d.pop("sip_username")

        sip_password = d.pop("sip_password")

        realm = d.pop("realm")

        softphone_password = cls(
            sip_username=sip_username,
            sip_password=sip_password,
            realm=realm,
        )


        softphone_password.additional_properties = d
        return softphone_password

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
