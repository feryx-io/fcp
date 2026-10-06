from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="AppDeviceResponse")



@_attrs_define
class AppDeviceResponse:
    """ The install's device and its SIP account: the password is shown here
    once, at each sign-in, and never again.

        Attributes:
            id (str):
            name (str):
            sip_username (str):
            sip_password (str):
            realm (str): The realm the password is hashed in, as the registrars challenge.
     """

    id: str
    name: str
    sip_username: str
    sip_password: str
    realm: str
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        name = self.name

        sip_username = self.sip_username

        sip_password = self.sip_password

        realm = self.realm


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "name": name,
            "sip_username": sip_username,
            "sip_password": sip_password,
            "realm": realm,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        name = d.pop("name")

        sip_username = d.pop("sip_username")

        sip_password = d.pop("sip_password")

        realm = d.pop("realm")

        app_device_response = cls(
            id=id,
            name=name,
            sip_username=sip_username,
            sip_password=sip_password,
            realm=realm,
        )


        app_device_response.additional_properties = d
        return app_device_response

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
