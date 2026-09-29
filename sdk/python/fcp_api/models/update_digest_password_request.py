from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset






T = TypeVar("T", bound="UpdateDigestPasswordRequest")



@_attrs_define
class UpdateDigestPasswordRequest:
    """ Update digest password request (for Cisco-style separate SIP credentials).

        Attributes:
            digest_password (str):
            store_for_provisioning (bool | Unset): Retain the plaintext SIP password for auto-provisioning templates.

                Defaults to true because most desk-phone provisioning templates require
                a raw SIP password rather than HA1/precomputed digest material.
     """

    digest_password: str
    store_for_provisioning: bool | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        digest_password = self.digest_password

        store_for_provisioning = self.store_for_provisioning


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "digest_password": digest_password,
        })
        if store_for_provisioning is not UNSET:
            field_dict["store_for_provisioning"] = store_for_provisioning

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        digest_password = d.pop("digest_password")

        store_for_provisioning = d.pop("store_for_provisioning", UNSET)

        update_digest_password_request = cls(
            digest_password=digest_password,
            store_for_provisioning=store_for_provisioning,
        )


        update_digest_password_request.additional_properties = d
        return update_digest_password_request

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
