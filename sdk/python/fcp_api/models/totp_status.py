from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="TotpStatus")



@_attrs_define
class TotpStatus:
    """ Whether the caller has a second factor, and whether their tenant
    requires one of them.

        Attributes:
            enabled (bool): Set up and confirmed: every password sign-in asks for a code.
            pending (bool): Set up, waiting for a code to confirm it.
            recovery_codes_left (int):
            required (bool): The tenant requires a second factor of the caller's role.
     """

    enabled: bool
    pending: bool
    recovery_codes_left: int
    required: bool
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        enabled = self.enabled

        pending = self.pending

        recovery_codes_left = self.recovery_codes_left

        required = self.required


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "enabled": enabled,
            "pending": pending,
            "recovery_codes_left": recovery_codes_left,
            "required": required,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        enabled = d.pop("enabled")

        pending = d.pop("pending")

        recovery_codes_left = d.pop("recovery_codes_left")

        required = d.pop("required")

        totp_status = cls(
            enabled=enabled,
            pending=pending,
            recovery_codes_left=recovery_codes_left,
            required=required,
        )


        totp_status.additional_properties = d
        return totp_status

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
