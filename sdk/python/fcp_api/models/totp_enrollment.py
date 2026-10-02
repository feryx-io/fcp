from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="TotpEnrollment")



@_attrs_define
class TotpEnrollment:
    """ A second factor being set up: the secret for the app, and the URI its
    QR code carries.

        Attributes:
            secret (str):
            otpauth_uri (str):
     """

    secret: str
    otpauth_uri: str
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        secret = self.secret

        otpauth_uri = self.otpauth_uri


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "secret": secret,
            "otpauth_uri": otpauth_uri,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        secret = d.pop("secret")

        otpauth_uri = d.pop("otpauth_uri")

        totp_enrollment = cls(
            secret=secret,
            otpauth_uri=otpauth_uri,
        )


        totp_enrollment.additional_properties = d
        return totp_enrollment

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
