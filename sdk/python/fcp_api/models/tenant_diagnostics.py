from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset






T = TypeVar("T", bound="TenantDiagnostics")



@_attrs_define
class TenantDiagnostics:
    """ A tenant's word on captures from its people's apps: off unless it turns
    them on, and then only for a user or device an administrator allows.

        Attributes:
            capture_allowed (bool | Unset): Captures may be allowed at all.
            capture_max_mb (int | Unset): The largest capture an app keeps or uploads, in MB (1 to 500).
            capture_keep_days (int | Unset): Days an uploaded capture is kept (1 to 365), unless a hold keeps it.
     """

    capture_allowed: bool | Unset = UNSET
    capture_max_mb: int | Unset = UNSET
    capture_keep_days: int | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        capture_allowed = self.capture_allowed

        capture_max_mb = self.capture_max_mb

        capture_keep_days = self.capture_keep_days


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if capture_allowed is not UNSET:
            field_dict["capture_allowed"] = capture_allowed
        if capture_max_mb is not UNSET:
            field_dict["capture_max_mb"] = capture_max_mb
        if capture_keep_days is not UNSET:
            field_dict["capture_keep_days"] = capture_keep_days

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        capture_allowed = d.pop("capture_allowed", UNSET)

        capture_max_mb = d.pop("capture_max_mb", UNSET)

        capture_keep_days = d.pop("capture_keep_days", UNSET)

        tenant_diagnostics = cls(
            capture_allowed=capture_allowed,
            capture_max_mb=capture_max_mb,
            capture_keep_days=capture_keep_days,
        )


        tenant_diagnostics.additional_properties = d
        return tenant_diagnostics

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
